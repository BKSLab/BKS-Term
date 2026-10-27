//! Bounded, text-only protocol for the optional screen adapter subprocess.
//! The worker returns a shape and selection, never terminal input or commands.

use std::io::{self, BufRead, Write};

use bks_terminal::{TerminalModel, TerminalSnapshot};
use serde::{Deserialize, Serialize};

use crate::tui_dialog::{self, Source, SUPPORTED_CODEX_VERSION};

pub(crate) const WORKER_ARGUMENT: &str = "--tui-adapter-worker";
pub(crate) const PROTOCOL_VERSION: u8 = 1;
pub(crate) const MAX_REQUEST_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_RESPONSE_BYTES: usize = 64 * 1024;
pub(crate) const MAX_CONTEXT_BYTES: usize = 24 * 1024;
const MAX_COLUMNS: usize = 512;
const MAX_ROWS: usize = 256;
const MAX_CELLS: usize = 65_536;
const MAX_LINE_BYTES: usize = 16 * 1024;
const MAX_TEXT_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub v: u8,
    pub id: u64,
    pub session: u64,
    pub revision: u64,
    pub input_revision: u64,
    pub cli_version: Option<String>,
    pub direct_codex_profile: bool,
    pub cols: usize,
    pub rows: usize,
    pub alternate: bool,
    pub lines: Vec<String>,
}

impl Request {
    /// Construct only from a live application screen. The host keeps the full
    /// original snapshot; this lossy text request is never an action binding.
    pub(crate) fn from_snapshot(
        id: u64,
        source: Source<'_>,
        screen: &TerminalSnapshot,
    ) -> io::Result<Self> {
        validate_dimensions(screen.cols, screen.rows)?;
        if screen.display_offset != 0
            || !source.active_session
            || !source.process_alive
            || screen.cells.len() != screen.cols * screen.rows
        {
            return Err(invalid("Adapter source is not a complete live screen"));
        }
        let mut total = 0usize;
        let mut lines = Vec::with_capacity(screen.rows);
        for row in screen.cells.chunks_exact(screen.cols) {
            let mut line = String::new();
            for cell in row {
                if line.len().saturating_add(cell.text.len()) > MAX_LINE_BYTES {
                    return Err(invalid("Adapter screen line is too long"));
                }
                line.push_str(&cell.text);
            }
            line.truncate(line.trim_end_matches(' ').len());
            total = total.saturating_add(line.len());
            if total > MAX_TEXT_BYTES {
                return Err(invalid("Adapter screen text is too large"));
            }
            lines.push(line);
        }
        let request = Self {
            v: PROTOCOL_VERSION,
            id,
            session: source.session_id,
            revision: source.screen_revision,
            input_revision: source.input_revision,
            cli_version: (!source.cli_version.is_empty()).then(|| source.cli_version.to_owned()),
            direct_codex_profile: source.direct_codex_profile,
            cols: screen.cols,
            rows: screen.rows,
            alternate: screen.alternate_screen,
            lines,
        };
        request.validate()?;
        Ok(request)
    }

    pub(crate) fn validate(&self) -> io::Result<()> {
        if self.v != PROTOCOL_VERSION {
            return Err(invalid("Unsupported adapter protocol version"));
        }
        validate_dimensions(self.cols, self.rows)?;
        if self.lines.len() != self.rows
            || self.cli_version.as_ref().is_some_and(|version| {
                version.is_empty() || version.len() > 64 || version.chars().any(char::is_control)
            })
        {
            return Err(invalid("Invalid adapter screen metadata"));
        }
        let mut total = 0usize;
        for line in &self.lines {
            total = total.saturating_add(line.len());
            if line.len() > MAX_LINE_BYTES
                || total > MAX_TEXT_BYTES
                || line.chars().any(char::is_control)
            {
                return Err(invalid("Invalid or oversized adapter screen text"));
            }
        }
        Ok(())
    }
}

fn validate_dimensions(cols: usize, rows: usize) -> io::Result<()> {
    if !(2..=MAX_COLUMNS).contains(&cols)
        || !(1..=MAX_ROWS).contains(&rows)
        || cols.checked_mul(rows).is_none_or(|cells| cells > MAX_CELLS)
    {
        return Err(invalid("Unsupported adapter screen dimensions"));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum Shape {
    #[serde(rename = "codex-folder-trust-v1")]
    CodexFolderTrust,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Match {
    pub shape: Shape,
    pub selected: usize,
    /// Full read-only context produced by the pinned adapter. Never input.
    pub context: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Response {
    pub v: u8,
    pub id: u64,
    pub session: u64,
    pub revision: u64,
    pub input_revision: u64,
    pub matched: Option<Match>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ready {
    v: u8,
    ready: bool,
}

pub(crate) fn decode_ready(frame: &[u8]) -> io::Result<()> {
    if frame.len() > MAX_RESPONSE_BYTES {
        return Err(invalid("Adapter greeting is too large"));
    }
    let ready: Ready =
        serde_json::from_slice(frame).map_err(|_| invalid("Invalid adapter greeting"))?;
    if ready.v != PROTOCOL_VERSION || !ready.ready {
        return Err(invalid("Unsupported adapter greeting"));
    }
    Ok(())
}

pub(crate) fn encode_request(request: &Request) -> io::Result<Vec<u8>> {
    request.validate()?;
    encode_frame(request, MAX_REQUEST_BYTES)
}

pub(crate) fn decode_response(frame: &[u8], request: &Request) -> io::Result<Response> {
    if frame.len() > MAX_RESPONSE_BYTES {
        return Err(invalid("Adapter response is too large"));
    }
    let response: Response =
        serde_json::from_slice(frame).map_err(|_| invalid("Invalid adapter response"))?;
    if response.v != PROTOCOL_VERSION
        || response.id != request.id
        || response.session != request.session
        || response.revision != request.revision
        || response.input_revision != request.input_revision
        || response.matched.as_ref().is_some_and(|matched| {
            matched.selected >= 2
                || matched.context.is_empty()
                || matched.context.len() > MAX_CONTEXT_BYTES
                || matched
                    .context
                    .chars()
                    .any(|character| character.is_control() && !matches!(character, '\r' | '\n'))
                || !request.direct_codex_profile
                || request.cli_version.as_deref() != Some(SUPPORTED_CODEX_VERSION)
        })
    {
        return Err(invalid("Adapter response does not match its source"));
    }
    Ok(response)
}

fn recognize(request: &Request) -> io::Result<Response> {
    request.validate()?;
    let mut response = Response {
        v: PROTOCOL_VERSION,
        id: request.id,
        session: request.session,
        revision: request.revision,
        input_revision: request.input_revision,
        matched: None,
    };
    if !request.direct_codex_profile
        || request.cli_version.as_deref() != Some(SUPPORTED_CODEX_VERSION)
    {
        return Ok(response);
    }

    // Plain, control-free text is reconstructed with the existing VT engine so
    // wide characters and combining marks keep valid cell boundaries. Only
    // these locally generated cursor-position sequences are interpreted.
    let mut terminal = TerminalModel::new(request.cols, request.rows);
    for (row, line) in request.lines.iter().enumerate() {
        terminal.push_bytes(format!("\x1b[{};1H", row + 1).as_bytes());
        terminal.push_bytes(line.as_bytes());
    }
    let mut screen = terminal.live_snapshot();
    screen.alternate_screen = request.alternate;
    for (row, expected) in screen.cells.chunks_exact(screen.cols).zip(&request.lines) {
        let text: String = row.iter().map(|cell| cell.text.as_str()).collect();
        if text.trim_end_matches(' ') != expected.trim_end_matches(' ') {
            return Err(invalid("Adapter screen text wraps or truncates"));
        }
    }
    let source = Source {
        session_id: request.session,
        screen_revision: request.revision,
        input_revision: request.input_revision,
        cli_version: SUPPORTED_CODEX_VERSION,
        direct_codex_profile: true,
        process_alive: true,
        active_session: true,
    };
    response.matched = tui_dialog::recognize(source, &screen).map(|dialog| Match {
        shape: Shape::CodexFolderTrust,
        selected: dialog.selected,
        context: dialog.context,
    });
    if response
        .matched
        .as_ref()
        .is_some_and(|matched| matched.context.len() > MAX_CONTEXT_BYTES)
    {
        return Err(invalid("Adapter dialog context is too large"));
    }
    Ok(response)
}

/// Invoke before normal argument parsing or any GUI/COM initialization. Failure
/// must terminate this subprocess without displaying an application error UI.
pub(crate) fn run_stdio() -> io::Result<()> {
    run_stream(io::stdin().lock(), io::stdout().lock())
}

fn run_stream(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    output.write_all(&encode_frame(
        &Ready {
            v: PROTOCOL_VERSION,
            ready: true,
        },
        MAX_RESPONSE_BYTES,
    )?)?;
    output.flush()?;
    while let Some(frame) = read_frame(&mut input, MAX_REQUEST_BYTES)? {
        let request: Request =
            serde_json::from_slice(&frame).map_err(|_| invalid("Invalid adapter request"))?;
        let response = recognize(&request)?;
        output.write_all(&encode_frame(&response, MAX_RESPONSE_BYTES)?)?;
        output.flush()?;
    }
    Ok(())
}

fn read_frame(input: &mut impl BufRead, limit: usize) -> io::Result<Option<Vec<u8>>> {
    let mut frame = Vec::new();
    loop {
        let buffer = input.fill_buf()?;
        if buffer.is_empty() {
            return if frame.is_empty() {
                Ok(None)
            } else {
                Err(invalid("Incomplete adapter JSON line"))
            };
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let count = newline.unwrap_or(buffer.len());
        if frame.len().saturating_add(count) > limit {
            return Err(invalid("Adapter JSON line is too large"));
        }
        frame.extend_from_slice(&buffer[..count]);
        input.consume(count + usize::from(newline.is_some()));
        if newline.is_some() {
            if frame.last() == Some(&b'\r') {
                frame.pop();
            }
            return Ok(Some(frame));
        }
    }
}

struct LimitedBuffer {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for LimitedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes.len().saturating_add(bytes.len()) > self.limit {
            return Err(invalid("Adapter JSON frame is too large"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn encode_frame(value: &impl Serialize, limit: usize) -> io::Result<Vec<u8>> {
    let mut buffer = LimitedBuffer {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut buffer, value).map_err(io::Error::other)?;
    buffer.bytes.push(b'\n');
    Ok(buffer.bytes)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const STARTUP: &str =
        include_str!("../tests/fixtures/tui/codex-0.164.0/startup-folder-trust-70x14.txt");

    fn request() -> Request {
        let mut terminal = TerminalModel::new(70, 14);
        terminal.push_bytes(
            STARTUP
                .trim_end_matches('\n')
                .replace('\n', "\r\n")
                .as_bytes(),
        );
        Request::from_snapshot(
            7,
            Source {
                session_id: 11,
                screen_revision: 24,
                input_revision: 0,
                cli_version: SUPPORTED_CODEX_VERSION,
                direct_codex_profile: true,
                process_alive: true,
                active_session: true,
            },
            &terminal.live_snapshot(),
        )
        .unwrap()
    }

    #[test]
    fn pinned_shape_round_trips_with_binding_and_read_only_context() {
        let request = request();
        let mut output = Vec::new();
        run_stream(Cursor::new(encode_request(&request).unwrap()), &mut output).unwrap();
        let mut frames = output.split(|byte| *byte == b'\n');
        decode_ready(frames.next().unwrap()).unwrap();
        let response = decode_response(frames.next().unwrap(), &request).unwrap();
        let matched = response.matched.unwrap();
        assert_eq!(matched.shape, Shape::CodexFolderTrust);
        assert_eq!(matched.selected, 0);
        assert!(matched.context.contains("/workspace/project"));
        assert!(matched.context.contains("Trust and continue"));
        assert_eq!(frames.next(), Some(b"".as_slice()));
        assert_eq!(frames.next(), None);
        let wire = String::from_utf8(output).unwrap();
        assert!(!wire.contains("commands"));
        assert!(!wire.contains("bytes"));
    }

    #[test]
    fn unknown_cli_version_and_changed_shape_stay_unmapped() {
        for version in [None, Some("0.164.1".to_owned()), Some("kimi".to_owned())] {
            let mut request = request();
            request.cli_version = version;
            assert_eq!(recognize(&request).unwrap().matched, None);
        }
        let mut request = request();
        request.direct_codex_profile = false;
        assert_eq!(recognize(&request).unwrap().matched, None);
        request.direct_codex_profile = true;
        request.lines[0].push_str(" changed");
        assert_eq!(recognize(&request).unwrap().matched, None);
    }

    #[test]
    fn unicode_path_preserves_combining_and_wide_text() {
        let mut request = request();
        let path = request
            .lines
            .iter_mut()
            .find(|line| line.contains("/workspace/project"))
            .unwrap();
        *path = "  C:\\Проект\\界e\u{301}".to_owned();
        assert!(recognize(&request).unwrap().matched.is_some());
    }

    #[test]
    fn rejects_protocol_bounds_controls_and_incomplete_screen() {
        let base = request();
        let mut changed = base.clone();
        changed.v += 1;
        assert!(changed.validate().is_err());
        changed = base.clone();
        changed.cols = usize::MAX;
        assert!(changed.validate().is_err());
        changed = base.clone();
        changed.lines.pop();
        assert!(changed.validate().is_err());
        changed = base.clone();
        changed.lines[0] = "x".repeat(MAX_LINE_BYTES + 1);
        assert!(changed.validate().is_err());
        changed = base;
        changed.lines[0] = "text\x1b[2J".to_owned();
        assert!(changed.validate().is_err());
    }

    #[test]
    fn rejects_unbound_response_and_extra_action_fields() {
        let request = request();
        let response = recognize(&request).unwrap();
        for field in ["v", "id", "session", "revision", "input_revision"] {
            let mut value = serde_json::to_value(&response).unwrap();
            value[field] = serde_json::json!(value[field].as_u64().unwrap() + 1);
            assert!(decode_response(&serde_json::to_vec(&value).unwrap(), &request).is_err());
        }
        let mut value = serde_json::to_value(&response).unwrap();
        value["matched"]["selected"] = serde_json::json!(2);
        assert!(decode_response(&serde_json::to_vec(&value).unwrap(), &request).is_err());
        value["matched"]["selected"] = serde_json::json!(0);
        value["matched"]["bytes"] = serde_json::json!("1\r");
        assert!(decode_response(&serde_json::to_vec(&value).unwrap(), &request).is_err());
        let mut value = serde_json::to_value(&response).unwrap();
        value["matched"]["shape"] = serde_json::json!("unknown-shape");
        assert!(decode_response(&serde_json::to_vec(&value).unwrap(), &request).is_err());
        value = serde_json::to_value(&response).unwrap();
        value["matched"]["context"] = serde_json::json!("x".repeat(MAX_CONTEXT_BYTES + 1));
        assert!(decode_response(&serde_json::to_vec(&value).unwrap(), &request).is_err());
        value["matched"]["context"] = serde_json::json!("context\0");
        assert!(decode_response(&serde_json::to_vec(&value).unwrap(), &request).is_err());
        assert!(decode_response(b"not json", &request).is_err());
        assert!(decode_ready(br#"{"v":2,"ready":true}"#).is_err());
        assert!(decode_ready(br#"{"v":1,"ready":true,"commands":[]}"#).is_err());
    }

    #[test]
    fn framing_limits_allocations_and_rejects_unterminated_input() {
        let mut input = Cursor::new(vec![b'x'; MAX_REQUEST_BYTES + 1]);
        assert!(read_frame(&mut input, MAX_REQUEST_BYTES).is_err());
        assert!(read_frame(&mut Cursor::new(b"{}"), MAX_REQUEST_BYTES).is_err());
        assert!(encode_frame(&"x".repeat(100), 10).is_err());
        let request = request();
        let mut value = serde_json::to_value(&request).unwrap();
        value["commands"] = serde_json::json!(["ignored"]);
        let mut bytes = serde_json::to_vec(&value).unwrap();
        bytes.push(b'\n');
        assert!(run_stream(Cursor::new(bytes), Vec::new()).is_err());
    }
}
