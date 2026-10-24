# BKS Term — память проекта (рабочее состояние)

**Обновлено:** 24 октября 2026 г.
**Назначение:** снимок текущего состояния для продолжения работы в новых сессиях агента. Обновлять после каждой завершённой итерации. Концепция и план — в `docs/concept.md` и `docs/setup-and-iterations.md`; при противоречии те документы важнее.

## 1. Где мы находимся

- **Текущий этап:** Этап 0 завершён. Создан каркас Cargo workspace (23.10): `crates/bks-pty` (lib), `crates/bks-app` (bin). Первый коммит `d864498` (main): документация + каркас.
- **Этап 1 в работе, ТОЧКА ОСТАНОВА 24.10:** нативное окно `BKS Term` запускает PowerShell через собственный PTY, выполняет однострочные и многострочные команды и поддерживает навигацию по результатам блоков: Ctrl+Up к последнему, Alt+Up/Down между ними, Ctrl+Shift+C копирует результат текущего блока. Реальные оконные сценарии проверены; ручная NVDA-проверка новых сочетаний — впереди.

## 1.1. Состояние кода bks-pty (важно при продолжении)

- `crates/bks-pty/src/lib.rs` — собственная обёртка ConPTY на `windows-rs 0.62.2`; выбор зафиксирован в `docs/decisions/0001-conpty-engine.md`. Исправлен запуск дочернего процесса: `STARTF_USESTDHANDLES` и недействительные stdio handles не дают выводу утечь в родительскую консоль.
- Зависимости bks-pty: production — только `windows 0.62.2`; `portable-pty 0.9.0` — dev-зависимость для spike.
- `crates/bks-pty/tests/conpty.rs` — 4 интеграционных теста (cmd echo, интерактивный ввод/resize, многострочный PowerShell, OSC 133;A/B prompt): **ВСЕ ПРОХОДЯТ** (24.10).
- `crates/bks-pty/examples/spawn_debug.rs` — отладочный пример (команда аргументом).
- `crates/bks-pty/examples/pty_spike.rs` — **spike portable-pty завершён успешно**: маркер получен через PTY, `cmd.exe` сам завершился с кодом 0. Чтение вынесено в поток, есть общий таймаут 10 с; на стартовый запрос `ESC[6n` пример отвечает `ESC[1;1R`. Проверка: `cargo run -p bks-pty --example pty_spike`.
- Рабочая директория: `C:\work\BKS.Lab\python\my_projects\BKS-Term` (git-репозиторий).

## 1.2. Состояние кода bks-app

- `crates/bks-app/src/main.rs` — нативное Win32-окно `BKS Term`: read-only EDIT для вывода, многострочный EDIT для команды, кнопка `Выполнить`, подписи, resize и минимальный размер 480x320. Enter выполняет команду, Shift+Enter вставляет новую строку. Tab/Shift+Tab явно переключают output/input/button.
- PowerShell стартует с `-NoLogo -NoProfile -NoExit -Command`; пользовательская функция `prompt` отправляет OSC 133;A перед видимым prompt и OSC 133;B после него. PSReadLine/OSC 133;C в режиме скринридера не сработал; политику выполнения не меняли. `WM_TIMER` каждые 50 мс забирает PTY-вывод; ввод идёт через `write_input` с CRLF, после успеха input очищается.
- `crates/bks-app/src/command_blocks.rs` хранит UTF-16-границы блоков, определяет начало результата по эху команды (включая continuation `>> `) и конец по следующему OSC 133;A. Ctrl+Up переводит фокус к началу последнего результата, Alt+Up/Down — к предыдущему/следующему, Ctrl+Shift+C выделяет результат и копирует его через `WM_COPY`. Alt-сочетания обрабатываются как `WM_SYSKEYDOWN`, не только `WM_KEYDOWN` (исправление после ручной проверки).
- До свежего OSC 133;B повторная отправка блокируется, текст команды остаётся в input; это предотвращает сдвиг anchors при быстром Enter во время выполнения. Для output установлен большой `EM_SETLIMITTEXT` вместо стандартных ~32 КБ.
- `target/tmp/check-output-blocks.ps1` подтвердил в реальном окне позиции caret, Alt+Up/Down через `WM_SYSKEYDOWN` при удержанном Alt, содержимое clipboard, многострочный `if` с завершающей пустой строкой, защиту быстрого Enter и вывод 104925 символов. Физическая клавиатурная инъекция стрелок в автоматизации не доставляется приложению, но пользователь вручную подтвердил работу Alt+Up/Down после исправления `WM_SYSKEYDOWN`; NVDA-проверка новых действий впереди.
- При добавлении output сохраняет `EM_GETSEL` и первую видимую строку, восстанавливает их через `EM_SETSEL`/`EM_LINESCROLL`; автопрокрутка только при курсоре в самом конце. Обновление обёрнуто в `WM_SETREDRAW`. `target/tmp/check-output-view.ps1` подтвердил сохранение позиции и follow-режим.
- `crates/bks-app/src/terminal_output.rs` — временный потоковый адаптер, не VT-рендерер: UTF-8 между чанками, удаление CSI/OSC/ESC, LF→CRLF и события OSC 133;A/B с UTF-16-смещением. Полноценные цвета, курсор, экранная модель и визуальная подсветка текущей строки не реализованы.
- Постоянная тёмная тема: фон `#0D1117`, surface `#010409`, текст `#7CFF9B`, приглушённый текст `#8B949E`, Consolas; тёмный DWM-заголовок и `DarkMode_Explorer` для output/input. Кнопка owner-draw с pressed/focus-состояниями. Пользователь подтвердил тему, Tab-навигацию и чтение начального живого вывода NVDA.
- Пользовательский логотип сохранён в `crates/bks-app/assets/bks-term-logo.png`; из левого знака подготовлены `bks-term-icon.png` и многоразмерная `bks-term.ico` (16–256). `build.rs` компилирует `app.rc` через `RC`/rc.exe из Windows SDK без новых Rust-зависимостей; иконка назначается классу и окну через `WM_SETICON`. `target/tmp/check-bks-icon.ps1` подтвердил `WM_GETICON` и извлечение large/small из release exe.
- Запись команды выполняется в UI-потоке: очень большие вставки могут ждать PTY. Esc закрывает окно — поведение ещё не исправлено.
- Release: `target/release/bks-app.exe`. Полный fmt/clippy/test/release набор прошёл 24.10: 19 unit-тестов + 4 PTY integration-теста (23 passed).

## 2. Среда разработки (установлено 23.10.2026)

| Компонент | Версия / состояние |
|---|---|
| Git | был установлен ранее |
| VS 2022 Build Tools | 17.14.39, workload VCTools (`--includeRecommended`), Windows SDK в комплекте |
| Rust | 1.99.1 stable, default host `x86_64-pc-windows-msvc` |
| Компоненты Rust | clippy, rustfmt — установлены и проверены |
| Smoke-тест | пройден: `cargo run` и release `.exe` работают (проект в `%TEMP%\bks-smoke`) |

Нюанс: в уже открытых терминалах `cargo` может не быть в PATH — полный путь `C:\Users\user\.cargo\bin`.

## 3. Инструмент оркестрации DeepSeek (скилл deepseek-coder)

- Скилл: `C:\Users\user\.kimi-code\skills\deepseek-coder\` (только для Kimi Code).
- Скрипт: `deepseek_codegen.py SPEC --out OUT [--max-tokens N]`, модель `deepseek/deepseek-v4.1-flash-1017` через Polza.ai.
- Ключ: `POLZA_API_KEY` (задан через setx 23.10). **Нюанс:** сессии Kimi, запущенные до setx, ключ напрямую не видят — инъекция из реестра:

  ```bash
  powershell -NoProfile -Command '$env:POLZA_API_KEY=[Environment]::GetEnvironmentVariable("POLZA_API_KEY","User"); python "C:/Users/user/.kimi-code/skills/deepseek-coder/deepseek_codegen.py" "<spec>" --out "<out>"'
  ```

- Скрипту нужны пути Windows-стиля (`C:/...`), не Git Bash `/tmp`.
- **Правило дробных ТЗ (после инцидента 23.10):** одно ТЗ = один модуль/класс/файл. Два первых запроса с ТЗ «на весь Этап 1» уходили в runaway-генерацию до потолка 131 072 выходных токенов (~18 ₽ за запрос). С 24.10 в скрипте `--max-tokens` по умолчанию **100000**, при обрезании ответа — код выхода 3.
- Черновики DeepSeek складывать в `.deepseek/` (не коммитить; добавить в `.gitignore` при создании workspace).

## 4. Текущие артефакты

- `.deepseek/spec-stage1.md` — ТЗ Этапа 1 **в старом монолитном виде (на все 5 файлов сразу)**. Перед использованием разбить на части: (1) workspace + `bks-pty`, (2) `bks-app`.
- `.deepseek/draft-stage1.md` — пустой (запросы были отменены/обрезаны), не использовать.

## 5. Планируемая структура workspace (Этап 1)

```text
Cargo.toml              # workspace, edition 2021, release: lto, codegen-units=1
crates/
  bks-pty/              # lib: обёртка ConPTY (CreatePseudoConsole, потоки чтения, resize, Drop)
  bks-app/              # bin: Win32-окно "BKS Term", многострочный вывод + поле ввода
```

Публичный API `bks-pty` зафиксирован в `.deepseek/spec-stage1.md` (PtySession: spawn/drain_output/write_input/resize/is_alive + Drop). Разрешённые зависимости Этапа 1: только `windows` crate + std.

## 6. Правила процесса (из плана разработки)

- Одна итерация = одна проверяемая возможность; каждая итерация завершается запускаемым `.exe`.
- После этапа: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --release`. Не писать «проверено», если не запускалось.
- NVDA-проверки — только вручную пользователем; агент не объявляет доступность пройденной.
- Стоп-критерий Этапа 1: если NVDA вообще не читает текст окна — не идти дальше, пересматривать GUI.

## 7. Решённая проблема ConPTY (24.10)

**Симптом:** вывод дочернего процесса утекал в родительскую консоль, а в stdout-pipe приходили только стартовые escape-последовательности.

**Причина и исправление:** в `STARTUPINFOEXW` не задавались `STARTF_USESTDHANDLES` и недействительные stdio handles. После исправления по образцу `portable-pty` маркер проходит через PTY, интерактивный ввод и resize работают в тестах.

**Проверки:** `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --release` — все прошли 24.10. Release: `target/release/bks-app.exe`.

**Архитектурное решение:** оставить собственный движок на `windows-rs`; `portable-pty` только для dev-spike. См. `docs/decisions/0001-conpty-engine.md`.

## 8. Расходы DeepSeek (23–24.10)

- 2 запуска монолитного ТЗ Этапа 1: по ~18 ₽ (runaway до 131K выходных токенов, отменены) — причина введения лимита и правила дробных ТЗ.
- ТЗ `bks-pty`: 1-я попытка 16K токенов ушла в reasoning без ответа (finish=length, ~2,2 ₽); 2-я с `--max-tokens 48000` — успех, 39715 токенов ≈ **5,4 ₽**.
- ТЗ `pty_spike`: запрос с `--max-tokens 40000` завершился `finish=length` без кода, 40000 выходных токенов, **3,87959415 ₽**; пример доработан вручную без повторного запроса.
- ТЗ `win32-window`: запрос с `--max-tokens 60000` также завершился `finish=length` без кода; окно реализовано вручную без повторного запроса.
- ТЗ `terminal_output`: успех с `--max-tokens 100000`, 8593 completion tokens, `finish=stop`, ~44 с; файл перенесён вручную, одна правка Clippy, 6 тестов проходят. Стоимость из API не получена.
- Согласованный лимит для следующих кодовых ТЗ: `--max-tokens 100000` (общий лимит reasoning + ответ). Это повышает шанс получить код, но не гарантирует его; без необходимости запросы не повторять.

## 9. Следующая итерация

Добавить историю команд: Up/Down в input, сохранение незавершённого черновика, Enter и кнопка по-прежнему выполняют команду. Отдельно спроектировать визуальную подсветку текущей строки output без потери доступности стандартного EDIT. Затем: resize PTY, полноценная VT/ANSI-модель и цвета; их не выдавать за решённые.
