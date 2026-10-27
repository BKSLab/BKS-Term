//! Deliberately small, versioned subset of the official Codex App Server API.
//! No I/O lives here. Only explicit caller actions create turns or decisions.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::Deserialize;
use serde_json::{json, Value};

pub const SUPPORTED_VERSION: &str = "0.164.0";
pub const MAX_FRAME_BYTES: usize = 128 * 1024;
pub const MAX_PROMPT_BYTES: usize = 16 * 1024;
pub const MAX_TRANSCRIPT_BYTES: usize = 192 * 1024;
pub const MAX_PENDING_APPROVALS: usize = 8;
pub const MAX_APPROVAL_BYTES: usize = 24 * 1024;
pub const TRANSCRIPT_TRUNCATION_MARKER: &str =
    "[Начало истории удалено: достигнут предел 192 КиБ.]\n";
const MAX_ITEM_IDS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Initializing,
    StartingThread,
    Ready,
    Busy,
    Failed,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    AcceptOnce,
    Decline,
    CancelTurn,
}

impl Decision {
    fn wire(self) -> &'static str {
        match self {
            Self::AcceptOnce => "accept",
            Self::Decline => "decline",
            Self::CancelTurn => "cancel",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Approval {
    pub request_id: Value,
    pub revision: u64,
    pub context: String,
    thread_id: String,
    turn_id: String,
    item_id: String,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub revision: u64,
    pub phase: Phase,
    pub status: String,
    pub transcript: String,
    /// Cumulative removed body length in normalized native EDIT UTF-16 units.
    /// The synthetic truncation marker is excluded from this coordinate.
    pub transcript_start: u64,
    pub approval: Option<Approval>,
    pub pending_count: usize,
    pub can_interrupt: bool,
    pub can_send: bool,
}

enum Request {
    Initialize,
    ThreadStart,
    TurnStart,
    Interrupt { turn_id: String },
}

pub struct Protocol {
    cwd: String,
    phase: Phase,
    status: String,
    transcript: String,
    transcript_start: u64,
    revision: u64,
    approval_revision: u64,
    next_id: u64,
    requests: BTreeMap<u64, Request>,
    retired_request_through: u64,
    thread_id: Option<String>,
    turn_id: Option<String>,
    interrupt_requested: bool,
    approvals: VecDeque<Approval>,
    streamed_items: BTreeSet<String>,
    last_agent_item: Option<String>,
}

impl Protocol {
    pub fn new(cwd: String) -> Self {
        Self {
            cwd,
            phase: Phase::Initializing,
            status: "Проверка версии и подключение…".to_owned(),
            transcript: String::new(),
            transcript_start: 0,
            revision: 1,
            approval_revision: 0,
            next_id: 1,
            requests: BTreeMap::new(),
            retired_request_through: 0,
            thread_id: None,
            turn_id: None,
            interrupt_requested: false,
            approvals: VecDeque::new(),
            streamed_items: BTreeSet::new(),
            last_agent_item: None,
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            revision: self.revision,
            phase: self.phase,
            status: self.status.clone(),
            transcript: self.transcript.clone(),
            transcript_start: self.transcript_start,
            approval: self.approvals.front().cloned(),
            pending_count: self.approvals.len(),
            can_interrupt: self.phase == Phase::Busy && !self.interrupt_requested,
            can_send: self.can_send(),
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn can_send(&self) -> bool {
        self.phase == Phase::Ready
            && !self
                .requests
                .values()
                .any(|request| matches!(request, Request::TurnStart))
    }

    pub fn initialize(&mut self) -> Value {
        self.request(
            "initialize",
            json!({"clientInfo":{"name":"bks_term","title":"BKS Term","version":env!("CARGO_PKG_VERSION")},
                "capabilities":{"experimentalApi":false}}),
            Request::Initialize,
        )
    }

    /// Validation failure never queues a prompt. There is no automatic retry.
    pub fn send_prompt(&mut self, text: &str) -> Result<Value, String> {
        if !self.can_send() {
            return Err("Сервер ещё не готов или предыдущий ответ не завершён.".to_owned());
        }
        if text.trim().is_empty() || text.len() > MAX_PROMPT_BYTES || text.contains('\0') {
            return Err("Введите сообщение до 16 КиБ без нулевых символов.".to_owned());
        }
        self.phase = Phase::Busy;
        self.turn_id = None;
        self.interrupt_requested = false;
        self.approvals.clear();
        self.streamed_items.clear();
        self.last_agent_item = None;
        self.set_status("Запрос отправлен. Ожидание ответа…");
        self.append(&format!("\nВы: {text}\n"));
        Ok(self.request(
            "turn/start",
            json!({"threadId":self.thread_id,"input":[{"type":"text","text":text}]}),
            Request::TurnStart,
        ))
    }

    pub fn interrupt(&mut self) -> Result<Option<Value>, String> {
        if self.phase != Phase::Busy || self.interrupt_requested {
            return Err("Нет активного запроса для прерывания.".to_owned());
        }
        self.interrupt_requested = true;
        self.approvals.clear();
        self.set_status("Запрошено прерывание. Ожидание подтверждения сервера…");
        // A user may interrupt before turn/start returned. Send once its exact
        // turn id is known, rather than guessing or discarding cancellation.
        Ok(self.interrupt_message())
    }

    fn interrupt_message(&mut self) -> Option<Value> {
        let turn_id = self.turn_id.clone()?;
        if self
            .requests
            .values()
            .any(|r| matches!(r, Request::Interrupt { turn_id: pending } if *pending == turn_id))
        {
            return None;
        }
        Some(self.request(
            "turn/interrupt",
            json!({"threadId":self.thread_id,"turnId":turn_id}),
            Request::Interrupt { turn_id },
        ))
    }

    /// Exact request identity, context and revision must still be current.
    pub fn decide(&mut self, shown: &Approval, decision: Decision) -> Result<Value, String> {
        if self.phase != Phase::Busy
            || self.interrupt_requested
            || self.approvals.front() != Some(shown)
            || self.thread_id.as_deref() != Some(&shown.thread_id)
            || self.turn_id.as_deref() != Some(&shown.turn_id)
        {
            return Err("Подтверждение устарело. Решение не отправлено.".to_owned());
        }
        self.approvals.pop_front();
        if decision == Decision::CancelTurn {
            self.approvals.clear();
            self.interrupt_requested = true;
        }
        self.set_status("Выбранное решение передано серверу.");
        Ok(json!({"id":shown.request_id,"result":{"decision":decision.wire()}}))
    }

    pub fn receive(&mut self, frame: &[u8]) -> Result<Vec<Value>, String> {
        if frame.len() > MAX_FRAME_BYTES {
            return Err("Сообщение App Server превышает 128 КиБ.".to_owned());
        }
        let message =
            parse_unique(frame).map_err(|_| "Некорректный JSON App Server.".to_owned())?;
        if !message.is_object()
            || message.get("jsonrpc").is_some_and(|v| v != "2.0")
            || message.get("method").is_some_and(|v| !v.is_string())
        {
            return Err("Несовместимый конверт JSON-RPC.".to_owned());
        }
        if let Some(method) = message.get("method").and_then(Value::as_str) {
            if message.get("result").is_some() || message.get("error").is_some() {
                return Err("Неоднозначный запрос JSON-RPC.".to_owned());
            }
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            if let Some(id) = message.get("id") {
                if !valid_id(id) {
                    return Err("Недопустимый request ID App Server.".to_owned());
                }
                return self.server_request(id.clone(), method, &params);
            }
            return self.notification(method, &params);
        }
        if message.get("result").is_some() == message.get("error").is_some() {
            return Err("Несовместимый ответ JSON-RPC.".to_owned());
        }
        let id = message
            .get("id")
            .and_then(Value::as_u64)
            .ok_or("Неверный ID ответа.")?;
        let request = match self.requests.remove(&id) {
            Some(request) => request,
            // Completed turns retire their outstanding interrupt IDs. Their
            // late success/error must not fail or change a newer active turn.
            None if id != 0 && id <= self.retired_request_through => return Ok(Vec::new()),
            None => return Err("Неизвестный ID ответа App Server.".to_owned()),
        };
        if matches!(&request, Request::Interrupt { turn_id }
            if self.phase != Phase::Busy || self.turn_id.as_deref() != Some(turn_id))
        {
            return Ok(Vec::new());
        }
        if let Some(error) = message.get("error") {
            let detail = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Ошибка JSON-RPC");
            return Err(format!("App Server: {}", bounded(detail, 4096)));
        }
        let result = &message["result"];
        match request {
            Request::Initialize => {
                if self.phase != Phase::Initializing
                    || !nonempty(&result["userAgent"])
                    || result["platformFamily"] != "windows"
                    || result["platformOs"] != "windows"
                    || !result["codexHome"].is_string()
                {
                    return Err(
                        "Схема initialize не соответствует поддерживаемой версии.".to_owned()
                    );
                }
                self.phase = Phase::StartingThread;
                self.set_status("Соединение установлено. Создание нового диалога…");
                let start = self.request(
                    "thread/start",
                    json!({"cwd":self.cwd,"approvalPolicy":"untrusted","approvalsReviewer":"user",
                        "sandbox":"read-only","ephemeral":true}),
                    Request::ThreadStart,
                );
                Ok(vec![json!({"method":"initialized","params":{}}), start])
            }
            Request::ThreadStart => {
                if self.phase != Phase::StartingThread
                    || !identifier(&result["thread"]["id"])
                    || result["approvalPolicy"] != "untrusted"
                    || result["approvalsReviewer"] != "user"
                    || result["sandbox"]["type"] != "readOnly"
                    || result["sandbox"]
                        .get("networkAccess")
                        .is_some_and(|v| v != false)
                    || !nonempty(&result["cwd"])
                    || !nonempty(&result["model"])
                {
                    return Err(
                        "Сервер не подтвердил поддерживаемую схему и ограничения диалога."
                            .to_owned(),
                    );
                }
                self.thread_id = result["thread"]["id"].as_str().map(str::to_owned);
                self.phase = Phase::Ready;
                self.set_status("Готово. Сообщение отправляется только кнопкой «Отправить».");
                self.append(&format!(
                    "Модель: {}\nПапка: {}\nSandbox: read-only; approvals: untrusted, user.\n",
                    result["model"].as_str().unwrap_or_default(),
                    result["cwd"].as_str().unwrap_or_default()
                ));
                Ok(Vec::new())
            }
            Request::TurnStart => {
                let turn = &result["turn"];
                if !identifier(&turn["id"]) {
                    return Err("Сервер не вернул ID запроса.".to_owned());
                }
                // Notifications can precede the response. A completed turn
                // must never become busy again because its response was late.
                if self.phase == Phase::Busy {
                    self.bind_turn(turn["id"].as_str().unwrap_or_default())?;
                    if self.interrupt_requested {
                        return Ok(self.interrupt_message().into_iter().collect());
                    }
                }
                self.revision += 1;
                Ok(Vec::new())
            }
            Request::Interrupt { .. } => Ok(Vec::new()),
        }
    }

    fn notification(&mut self, method: &str, params: &Value) -> Result<Vec<Value>, String> {
        if method == "serverRequest/resolved" {
            if params["threadId"].as_str() == self.thread_id.as_deref() {
                if !valid_id(&params["requestId"]) {
                    return Err("Сервер прислал resolved без точного request ID.".to_owned());
                }
                let before = self.approvals.len();
                self.approvals
                    .retain(|a| a.request_id != params["requestId"]);
                if before != self.approvals.len() {
                    self.set_status(
                        "Сервер закрыл запрос подтверждения. Предыдущий выбор недействителен.",
                    );
                }
            }
            return Ok(Vec::new());
        }
        if params["threadId"].as_str() != self.thread_id.as_deref() {
            return Ok(Vec::new());
        }
        if method == "thread/closed" || method == "thread/archived" {
            return Err("Диалог закрыт сервером.".to_owned());
        }
        if self.phase != Phase::Busy {
            return Ok(Vec::new());
        }
        if matches!(method, "turn/started" | "turn/completed") {
            let turn = &params["turn"];
            let id = turn["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or("Не указан turn ID.")?;
            self.bind_turn(id)?;
            if method == "turn/completed" {
                let status = turn["status"].as_str().ok_or("Не указан статус turn.")?;
                self.approvals.clear();
                let mut retired = self.retired_request_through;
                self.requests.retain(|request_id, request| {
                    if matches!(request, Request::Interrupt { turn_id } if turn_id == id) {
                        retired = retired.max(*request_id);
                        false
                    } else {
                        true
                    }
                });
                self.retired_request_through = retired;
                self.phase = Phase::Ready;
                self.interrupt_requested = false;
                match status {
                    "completed" => self.set_status("Ответ завершён."),
                    "interrupted" => self.set_status("Запрос прерван сервером."),
                    "failed" => {
                        self.set_status(
                            "Запрос завершился ошибкой. Автоматический повтор отключён.",
                        );
                        let error = turn.get("error").unwrap_or(&Value::Null).to_string();
                        self.append(&format!("\nОшибка: {}\n", bounded(&error, 4096)));
                    }
                    _ => return Err("Неизвестный конечный статус turn.".to_owned()),
                }
                self.append("\n");
            } else if self.interrupt_requested {
                return Ok(self.interrupt_message().into_iter().collect());
            }
            return Ok(Vec::new());
        }
        if params["turnId"].as_str() != self.turn_id.as_deref() {
            return Ok(Vec::new());
        }
        match method {
            "item/agentMessage/delta" => {
                if !identifier(&params["itemId"]) {
                    return Err("Не указан поддерживаемый item ID.".to_owned());
                }
                let item_id = params["itemId"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or("Не указан item ID.")?;
                let delta = params["delta"]
                    .as_str()
                    .ok_or("Некорректный фрагмент ответа.")?;
                if self.streamed_items.len() >= MAX_ITEM_IDS
                    && !self.streamed_items.contains(item_id)
                {
                    return Err("Слишком много сообщений в одном ответе.".to_owned());
                }
                self.streamed_items.insert(item_id.to_owned());
                if self.last_agent_item.as_deref() != Some(item_id) {
                    self.append("\nCodex: ");
                    self.last_agent_item = Some(item_id.to_owned());
                }
                self.append(delta);
            }
            "item/completed" => {
                let item = &params["item"];
                if !identifier(&item["id"]) {
                    return Err("Сервер завершил действие без точного item ID.".to_owned());
                }
                if item["type"] == "agentMessage" {
                    let id = item["id"].as_str().ok_or("Не указан ID сообщения.")?;
                    if !self.streamed_items.contains(id) {
                        let text = item["text"].as_str().ok_or("Не указан текст сообщения.")?;
                        self.append(&format!("\nCodex: {text}\n"));
                    }
                }
                // Any completed item invalidates its pending approvals.
                let before = self.approvals.len();
                self.approvals
                    .retain(|a| Some(a.item_id.as_str()) != item["id"].as_str());
                if before != self.approvals.len() {
                    self.set_status("Действие завершено сервером. Подтверждение закрыто.");
                }
            }
            "error" => {
                let error = params["error"]["message"]
                    .as_str()
                    .unwrap_or("Ошибка App Server");
                self.append(&format!("\nОшибка: {}\n", bounded(error, 4096)));
                self.set_status(
                    "Сервер сообщил об ошибке. BKS Term повторный запрос не отправляет.",
                );
            }
            _ => {}
        }
        Ok(Vec::new())
    }

    fn bind_turn(&mut self, id: &str) -> Result<(), String> {
        if id.is_empty() || id.len() > 1024 || id.contains('\0') {
            return Err("Неподдерживаемый turn ID.".to_owned());
        }
        if self.turn_id.as_deref().is_some_and(|current| current != id) {
            return Err(
                "Сервер сменил активный turn ID без нового запроса пользователя.".to_owned(),
            );
        }
        self.turn_id = Some(id.to_owned());
        Ok(())
    }

    fn server_request(
        &mut self,
        id: Value,
        method: &str,
        params: &Value,
    ) -> Result<Vec<Value>, String> {
        if self.approvals.iter().any(|a| a.request_id == id) {
            return Err("Повторный request ID: подтверждения инвалидированы.".to_owned());
        }
        let fields = [
            "approvalId",
            "command",
            "commandActions",
            "cwd",
            "environmentId",
            "itemId",
            "kind",
            "networkApprovalContext",
            "proposedExecpolicyAmendment",
            "proposedNetworkPolicyAmendments",
            "reason",
            "startedAtMs",
            "threadId",
            "turnId",
        ];
        let compatible = method == "item/commandExecution/requestApproval"
            && self.phase == Phase::Busy
            && !self.interrupt_requested
            && self.thread_id.as_deref() == params["threadId"].as_str()
            && self.turn_id.as_deref() == params["turnId"].as_str()
            && identifier(&params["itemId"])
            && nonempty(&params["command"])
            && nonempty(&params["cwd"])
            && params
                .get("reason")
                .is_none_or(|reason| reason.is_null() || reason.is_string())
            && params
                .get("approvalId")
                .is_none_or(|approval| approval.is_null() || identifier(approval))
            && params
                .get("commandActions")
                .is_none_or(|actions| actions.is_null() || actions.is_array())
            && params["startedAtMs"].is_i64()
            && params.get("kind").is_none_or(|kind| kind == "command")
            && params
                .get("networkApprovalContext")
                .is_none_or(Value::is_null)
            && params
                .get("proposedExecpolicyAmendment")
                .is_none_or(Value::is_null)
            && params
                .get("proposedNetworkPolicyAmendments")
                .is_none_or(Value::is_null)
            && params.get("environmentId").is_none_or(Value::is_null)
            && params
                .as_object()
                .is_some_and(|object| object.keys().all(|k| fields.contains(&k.as_str())));
        let context = format!(
            "Метод: {method}\nRequest ID: {id}\n{}",
            serde_json::to_string_pretty(params).map_err(|e| e.to_string())?
        );
        if !compatible || context.len() > MAX_APPROVAL_BYTES {
            self.set_status("Неподдерживаемый запрос App Server. Подтверждение не отправлено.");
            self.append(&format!(
                "\nНеподдерживаемый запрос (без подтверждения): {}\n",
                bounded(&context, MAX_APPROVAL_BYTES)
            ));
            return Ok(vec![json!({"id":id,"error":{"code":-32601,
                "message":"BKS Term does not support this request or its context; no approval was granted"}})]);
        }
        if self.approvals.len() >= MAX_PENDING_APPROVALS {
            return Err("Превышен предел 8 ожидающих подтверждений. Соединение закрывается без подтверждения.".to_owned());
        }
        self.approval_revision += 1;
        self.approvals.push_back(Approval {
            request_id: id,
            revision: self.approval_revision,
            context,
            thread_id: params["threadId"].as_str().unwrap_or_default().to_owned(),
            turn_id: params["turnId"].as_str().unwrap_or_default().to_owned(),
            item_id: params["itemId"].as_str().unwrap_or_default().to_owned(),
        });
        self.set_status("Требуется решение пользователя. Прочитайте полный контекст; вариант по умолчанию не выбран.");
        Ok(Vec::new())
    }

    pub fn fail(&mut self, detail: &str) {
        self.approvals.clear();
        self.requests.clear();
        self.phase = Phase::Failed;
        self.status = bounded(detail, 4096).to_owned();
        self.append(&format!(
            "\nСоединение остановлено: {}\n",
            bounded(detail, 4096)
        ));
    }

    pub fn stop(&mut self) {
        self.approvals.clear();
        self.requests.clear();
        self.phase = Phase::Stopped;
        self.set_status("App Server остановлен.");
    }

    pub fn action_error(&mut self, detail: &str) {
        self.set_status(detail);
    }

    fn request(&mut self, method: &str, params: Value, kind: Request) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.requests.insert(id, kind);
        json!({"id":id,"method":method,"params":params})
    }

    fn set_status(&mut self, status: &str) {
        self.status = bounded(status, 4096).replace('\0', "�");
        self.revision += 1;
    }

    fn append(&mut self, text: &str) {
        let text = text.replace('\0', "�");
        self.transcript.push_str(&text);
        if self.transcript.len() > MAX_TRANSCRIPT_BYTES {
            let marker = TRANSCRIPT_TRUNCATION_MARKER;
            let previous_marker = if self.transcript_start == 0 {
                0
            } else {
                marker.len()
            };
            let mut remove = self.transcript.len() - MAX_TRANSCRIPT_BYTES + marker.len();
            while !self.transcript.is_char_boundary(remove) {
                remove += 1;
            }
            // Keep complete nearby logical lines, preserving wrapped native
            // EDIT rows for surviving text. Very long lines still have the
            // strict byte bound; never split a Unicode scalar or CRLF pair.
            if let Some(newline) = self.transcript[remove..].find('\n') {
                if newline < 4096 {
                    remove += newline + 1;
                }
            }
            if self.transcript.as_bytes().get(remove.wrapping_sub(1)) == Some(&b'\r')
                && self.transcript.as_bytes().get(remove) == Some(&b'\n')
            {
                remove += 1;
            }
            self.transcript_start += native_utf16_units(&self.transcript[previous_marker..remove]);
            self.transcript.drain(..remove);
            self.transcript.insert_str(0, marker);
        }
        self.revision += 1;
    }
}

fn native_utf16_units(text: &str) -> u64 {
    let mut units = 0;
    let mut previous_cr = false;
    for character in text.chars() {
        units += match character {
            '\n' if previous_cr => 0,
            '\r' | '\n' => 2,
            _ => character.len_utf16() as u64,
        };
        previous_cr = character == '\r';
    }
    units
}

fn nonempty(value: &Value) -> bool {
    value.as_str().is_some_and(|value| {
        !value.trim().is_empty() && !value.contains('\0') && value.len() <= 16 * 1024
    })
}

fn valid_id(id: &Value) -> bool {
    id.is_i64()
        || id.is_u64()
        || id
            .as_str()
            .is_some_and(|id| id.len() <= 1024 && !id.contains('\0'))
}

fn identifier(value: &Value) -> bool {
    nonempty(value) && value.as_str().is_some_and(|id| id.len() <= 1024)
}

fn bounded(text: &str, limit: usize) -> &str {
    let mut end = limit.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Reject duplicate keys at every depth; silently choosing one command or ID
/// would turn an ambiguous approval into a seemingly precise native dialog.
fn parse_unique(bytes: &[u8]) -> serde_json::Result<Value> {
    struct Unique(Value);
    impl<'de> Deserialize<'de> for Unique {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct Visitor;
            impl<'de> serde::de::Visitor<'de> for Visitor {
                type Value = Unique;
                fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    formatter.write_str("JSON without duplicate object keys")
                }
                fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Unique, E> {
                    Ok(Unique(v.into()))
                }
                fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Unique, E> {
                    Ok(Unique(v.into()))
                }
                fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Unique, E> {
                    Ok(Unique(v.into()))
                }
                fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Unique, E> {
                    serde_json::Number::from_f64(v)
                        .map(|v| Unique(v.into()))
                        .ok_or_else(|| E::custom("invalid number"))
                }
                fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Unique, E> {
                    Ok(Unique(v.into()))
                }
                fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Unique, E> {
                    Ok(Unique(v.into()))
                }
                fn visit_none<E: serde::de::Error>(self) -> Result<Unique, E> {
                    Ok(Unique(Value::Null))
                }
                fn visit_unit<E: serde::de::Error>(self) -> Result<Unique, E> {
                    Ok(Unique(Value::Null))
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    self,
                    mut seq: A,
                ) -> Result<Unique, A::Error> {
                    let mut values = Vec::new();
                    while let Some(Unique(v)) = seq.next_element()? {
                        values.push(v);
                    }
                    Ok(Unique(Value::Array(values)))
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> Result<Unique, A::Error> {
                    let mut values = serde_json::Map::new();
                    while let Some(key) = map.next_key::<String>()? {
                        if values.contains_key(&key) {
                            return Err(serde::de::Error::custom("duplicate key"));
                        }
                        let Unique(value) = map.next_value()?;
                        values.insert(key, value);
                    }
                    Ok(Unique(Value::Object(values)))
                }
            }
            deserializer.deserialize_any(Visitor)
        }
    }
    serde_json::from_slice::<Unique>(bytes).map(|Unique(value)| value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receive(protocol: &mut Protocol, value: Value) -> Vec<Value> {
        protocol
            .receive(&serde_json::to_vec(&value).unwrap())
            .unwrap()
    }

    fn ready() -> Protocol {
        let mut p = Protocol::new(r"C:\fixture".to_owned());
        assert_eq!(p.initialize()["id"], 1);
        let outputs = receive(
            &mut p,
            json!({"id":1,"result":{"codexHome":"C:\\fixture-home","userAgent":"fixture/0.164.0","platformFamily":"windows","platformOs":"windows"}}),
        );
        assert_eq!(outputs[0]["method"], "initialized");
        assert_eq!(outputs[1]["method"], "thread/start");
        assert_eq!(outputs[1]["params"]["approvalsReviewer"], "user");
        assert_eq!(outputs[1]["params"]["sandbox"], "read-only");
        receive(
            &mut p,
            json!({"id":2,"result":{"thread":{"id":"thread-1"},"approvalPolicy":"untrusted","approvalsReviewer":"user","sandbox":{"type":"readOnly","networkAccess":false},"cwd":"C:\\fixture","model":"fixture-model"}}),
        );
        p
    }

    fn busy() -> Protocol {
        let mut p = ready();
        assert_eq!(
            p.send_prompt("fixture prompt").unwrap()["method"],
            "turn/start"
        );
        receive(
            &mut p,
            json!({"id":3,"result":{"turn":{"id":"turn-1","status":"inProgress"}}}),
        );
        p
    }

    fn approval(id: Value) -> Value {
        json!({"id":id,"method":"item/commandExecution/requestApproval","params":{
            "threadId":"thread-1","turnId":"turn-1","itemId":"command-1","startedAtMs":123,
            "command":"Get-ChildItem","cwd":"C:\\fixture","reason":"fixture reason","kind":"command"}})
    }

    #[test]
    fn handshake_never_sends_a_prompt_and_validates_policy() {
        let p = ready();
        assert_eq!(p.snapshot().phase, Phase::Ready);
        assert!(!p.snapshot().transcript.contains("Вы:"));
        let mut p = Protocol::new("C:\\fixture".to_owned());
        p.initialize();
        assert!(p
            .receive(br#"{"id":1,"result":{"userAgent":"other-version"}}"#)
            .is_err());
    }

    #[test]
    fn strict_json_rejects_duplicate_fields_and_invalid_envelopes() {
        for frame in [
            br#"{"method":"foo","method":"bar"}"#.as_slice(),
            br#"{"method":"foo","id":1,"params":{"command":"safe","command":"unsafe"}}"#,
            br#"{"method":"foo","id":1,"result":{}}"#,
            br#"{"jsonrpc":"1.0","method":"foo"}"#,
            br#"{"id":1,"result":{},"error":{}}"#,
            br#"{"method":"foo","id":1.5}"#,
            b"[]",
            b"{",
        ] {
            assert!(busy().receive(frame).is_err(), "{frame:?}");
        }
    }

    #[test]
    fn prompt_requires_ready_and_explicit_call_and_is_bounded() {
        let mut p = Protocol::new("C:\\fixture".to_owned());
        assert!(p.send_prompt("not ready").is_err());
        let mut p = ready();
        for invalid in [
            String::new(),
            " ".to_owned(),
            "x\0y".to_owned(),
            "x".repeat(MAX_PROMPT_BYTES + 1),
        ] {
            assert!(p.send_prompt(&invalid).is_err());
        }
        let request = p.send_prompt("Только явный запрос").unwrap();
        assert_eq!(request["params"]["input"][0]["text"], "Только явный запрос");
        assert!(p.send_prompt("second turn").is_err());
    }

    #[test]
    fn stream_and_final_without_duplicate_text_and_unrelated_events() {
        let mut p = busy();
        receive(
            &mut p,
            json!({"method":"item/agentMessage/delta","params":{"threadId":"wrong","turnId":"turn-1","itemId":"a","delta":"wrong"}}),
        );
        receive(
            &mut p,
            json!({"method":"item/agentMessage/delta","params":{"threadId":"thread-1","turnId":"turn-1","itemId":"a","delta":"Привет"}}),
        );
        receive(
            &mut p,
            json!({"method":"item/completed","params":{"threadId":"thread-1","turnId":"turn-1","item":{"type":"agentMessage","id":"a","text":"Привет"}}}),
        );
        receive(
            &mut p,
            json!({"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"completed"}}}),
        );
        assert_eq!(p.snapshot().transcript.matches("Привет").count(), 1);
        assert!(!p.snapshot().transcript.contains("wrong"));
        assert_eq!(p.snapshot().phase, Phase::Ready);
    }

    #[test]
    fn explicit_interrupt_waits_for_exact_turn_id_and_is_sent_once() {
        let mut p = ready();
        p.send_prompt("fixture").unwrap();
        assert!(p.interrupt().unwrap().is_none());
        let messages = receive(
            &mut p,
            json!({"method":"turn/started","params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"inProgress"}}}),
        );
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["method"], "turn/interrupt");
        assert_eq!(messages[0]["params"]["turnId"], "turn-1");
        let messages = receive(&mut p, json!({"id":3,"result":{"turn":{"id":"turn-1"}}}));
        assert!(messages.is_empty());
        assert!(p.interrupt().is_err());
    }

    #[test]
    fn approval_preserves_exact_string_or_number_id_and_context() {
        for id in [json!(77), json!("server-77")] {
            let mut p = busy();
            assert!(receive(&mut p, approval(id.clone())).is_empty());
            let shown = p.snapshot().approval.unwrap();
            assert_eq!(shown.request_id, id);
            assert!(shown.context.contains("Get-ChildItem"));
            assert!(shown.context.contains("fixture reason"));
            let reply = p.decide(&shown, Decision::AcceptOnce).unwrap();
            assert_eq!(reply, json!({"id":id,"result":{"decision":"accept"}}));
            assert!(p.decide(&shown, Decision::AcceptOnce).is_err());
        }
    }

    #[test]
    fn resolved_completed_interrupted_or_changed_approval_cannot_be_applied() {
        for event in [
            json!({"method":"serverRequest/resolved","params":{"threadId":"thread-1","requestId":77}}),
            json!({"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"interrupted"}}}),
            json!({"method":"item/completed","params":{"threadId":"thread-1","turnId":"turn-1","item":{"type":"commandExecution","id":"command-1"}}}),
        ] {
            let mut p = busy();
            receive(&mut p, approval(json!(77)));
            let shown = p.snapshot().approval.unwrap();
            receive(&mut p, event);
            assert!(p.decide(&shown, Decision::AcceptOnce).is_err());
        }
        let mut p = busy();
        receive(&mut p, approval(json!(77)));
        let mut shown = p.snapshot().approval.unwrap();
        shown.context.push_str(" changed");
        assert!(p.decide(&shown, Decision::AcceptOnce).is_err());
        shown = p.snapshot().approval.unwrap();
        shown.revision += 1;
        assert!(p.decide(&shown, Decision::Decline).is_err());
        p.interrupt().unwrap();
        assert!(p
            .decide(
                &p.snapshot().approval.unwrap_or(shown),
                Decision::CancelTurn
            )
            .is_err());
    }

    #[test]
    fn unsupported_or_ambiguous_approval_never_produces_acceptance() {
        for mutation in [
            "method",
            "network",
            "kind",
            "amendment",
            "unknown",
            "missing-command",
            "wrong-turn",
            "too-large",
        ] {
            let mut p = busy();
            let mut request = approval(json!(77));
            match mutation {
                "method" => request["method"] = json!("item/fileChange/requestApproval"),
                "network" => {
                    request["params"]["networkApprovalContext"] = json!({"host":"fixture"})
                }
                "kind" => request["params"]["kind"] = json!("stdin"),
                "amendment" => {
                    request["params"]["proposedExecpolicyAmendment"] = json!(["fixture"])
                }
                "unknown" => request["params"]["additionalPermissions"] = json!({}),
                "missing-command" => request["params"]["command"] = Value::Null,
                "wrong-turn" => request["params"]["turnId"] = json!("old"),
                _ => request["params"]["reason"] = json!("x".repeat(MAX_APPROVAL_BYTES)),
            }
            let outputs = receive(&mut p, request);
            assert_eq!(outputs.len(), 1);
            assert!(outputs[0].get("error").is_some());
            assert!(outputs[0].get("result").is_none());
            assert!(p.snapshot().approval.is_none());
        }
    }

    #[test]
    fn approvals_are_bounded_and_duplicate_id_is_fatal() {
        let mut p = busy();
        receive(&mut p, approval(json!(77)));
        assert!(p
            .receive(&serde_json::to_vec(&approval(json!(77))).unwrap())
            .is_err());
        for id in 78..(78 + MAX_PENDING_APPROVALS - 1) {
            receive(&mut p, approval(json!(id)));
        }
        assert_eq!(p.snapshot().pending_count, MAX_PENDING_APPROVALS);
        assert!(p
            .receive(&serde_json::to_vec(&approval(json!(1000))).unwrap())
            .is_err());
        p.fail("fixture overflow");
        assert!(p.snapshot().approval.is_none());
    }

    #[test]
    fn transcript_is_bounded_at_unicode_boundaries() {
        let mut p = busy();
        for _ in 0..20 {
            p.append(&"я".repeat(20 * 1024));
        }
        assert!(p.snapshot().transcript.len() <= MAX_TRANSCRIPT_BYTES);
        assert!(p
            .snapshot()
            .transcript
            .starts_with("[Начало истории удалено:"));
        assert!(p.receive(&vec![b' '; MAX_FRAME_BYTES + 1]).is_err());
    }

    #[test]
    fn failed_turn_is_not_success_and_is_never_retried() {
        let mut p = busy();
        let output = receive(
            &mut p,
            json!({"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"failed","error":{"message":"fixture failure"}}}}),
        );
        assert!(output.is_empty());
        assert!(p.snapshot().transcript.contains("fixture failure"));
        assert!(p.snapshot().status.contains("ошибкой"));
    }

    #[test]
    fn cancel_decision_invalidates_every_other_pending_approval() {
        let mut p = busy();
        receive(&mut p, approval(json!(77)));
        receive(&mut p, approval(json!(78)));
        let shown = p.snapshot().approval.unwrap();
        let reply = p.decide(&shown, Decision::CancelTurn).unwrap();
        assert_eq!(reply["result"]["decision"], "cancel");
        assert_eq!(p.snapshot().pending_count, 0);
        assert!(!p.snapshot().can_interrupt);
        assert!(p.decide(&shown, Decision::AcceptOnce).is_err());
    }

    #[test]
    fn late_start_response_cannot_bind_a_new_turn_to_the_previous_id() {
        let mut p = ready();
        p.send_prompt("first fixture").unwrap();
        receive(
            &mut p,
            json!({"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"completed"}}}),
        );
        assert!(!p.snapshot().can_send);
        assert!(p.send_prompt("too early").is_err());
        receive(&mut p, json!({"id":3,"result":{"turn":{"id":"turn-1"}}}));
        assert!(p.snapshot().can_send);
        assert!(p.send_prompt("next explicit fixture").is_ok());
    }

    #[test]
    fn delayed_interrupt_ack_for_completed_a_does_not_block_or_fail_b() {
        for error in [false, true] {
            let mut p = busy();
            let interrupt_a = p.interrupt().unwrap().unwrap();
            receive(
                &mut p,
                json!({"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"interrupted"}}}),
            );
            assert!(p.requests.is_empty());
            let start_b = p.send_prompt("explicit B").unwrap();
            receive(
                &mut p,
                json!({"id":start_b["id"],"result":{"turn":{"id":"turn-B"}}}),
            );
            let interrupt_b = p
                .interrupt()
                .unwrap()
                .expect("B must get its own interrupt");
            assert_eq!(interrupt_b["params"]["turnId"], "turn-B");
            assert_ne!(interrupt_a["id"], interrupt_b["id"]);
            let before = p.snapshot();
            let late = if error {
                json!({"id":interrupt_a["id"],"error":{"code":-32600,"message":"A is already completed"}})
            } else {
                json!({"id":interrupt_a["id"],"result":{}})
            };
            assert!(receive(&mut p, late).is_empty());
            assert_eq!(p.snapshot().phase, Phase::Busy);
            assert_eq!(p.snapshot().status, before.status);
            assert_eq!(p.snapshot().revision, before.revision);
            assert!(p.requests.values().any(
                |request| matches!(request,Request::Interrupt{turn_id} if turn_id == "turn-B")
            ));
        }
    }

    #[test]
    fn transcript_start_counts_only_removed_body_in_native_unicode_crlf_units() {
        let mut p = Protocol::new("C:\\fixture".to_owned());
        let mut index = 0;
        loop {
            let line = format!("Строка {index:05} Ж😀e\u{301}\r\n");
            if p.transcript.len() + line.len() > MAX_TRANSCRIPT_BYTES {
                break;
            }
            p.append(&line);
            index += 1;
        }
        assert_eq!(p.snapshot().transcript_start, 0);
        for pass in 0..2 {
            let old = p.snapshot();
            let addition = (0..1800)
                .map(|i| format!("Добавлено {pass}-{i:05} Ж😀e\u{301}\r\n"))
                .collect::<String>();
            p.append(&addition);
            let new = p.snapshot();
            let old_body = if old.transcript_start == 0 {
                old.transcript.as_str()
            } else {
                old.transcript
                    .strip_prefix(TRANSCRIPT_TRUNCATION_MARKER)
                    .unwrap()
            };
            let new_body = new
                .transcript
                .strip_prefix(TRANSCRIPT_TRUNCATION_MARKER)
                .unwrap();
            let first_line = new_body.split_inclusive('\n').next().unwrap();
            let removed = old_body
                .find(first_line)
                .expect("surviving line belongs to old body");
            assert!(removed > 0);
            // This fixture uses CRLF already, so native UTF-16 is an
            // independent exact reference, including surrogate/combining text.
            assert_eq!(
                new.transcript_start - old.transcript_start,
                old_body[..removed].encode_utf16().count() as u64
            );
            assert!(new.transcript.len() <= MAX_TRANSCRIPT_BYTES);
            assert!(new.transcript.ends_with(&addition));
        }
    }
}
