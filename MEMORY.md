# BKS Term — память проекта (рабочее состояние)

**Обновлено:** 23 октября 2026 г.
**Назначение:** снимок текущего состояния для продолжения работы в новых сессиях агента. Обновлять после каждой завершённой итерации. Концепция и план — в `docs/concept.md` и `docs/setup-and-iterations.md`; при противоречии те документы важнее.

## 1. Где мы находимся

- **Текущий этап:** Этап 0 завершён. Создан каркас Cargo workspace (23.10): `crates/bks-pty` (lib, пока только тип `PtySize` + тест), `crates/bks-app` (bin-заглушка). Все проверки проходят: fmt, clippy `-D warnings`, test, release build → `target/release/bks-app.exe`.
- **Готов к Этапу 1:** окно Win32 + минимальный ConPTY с PowerShell. Реализацию `PtySession` (API зафиксирован в `.deepseek/spec-stage1.md`) писать в `bks-pty`.
- Рабочая директория: `C:\work\BKS.Lab\python\my_projects\BKS-Term` (git-репозиторий).

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
- **Правило дробных ТЗ (после инцидента 23.10):** одно ТЗ = один модуль/класс/файл. Два первых запроса с ТЗ «на весь Этап 1» уходили в runaway-генерацию до потолка 131 072 выходных токенов (~18 ₽ за запрос). Теперь в скрипте `--max-tokens` по умолчанию 16000 (≈ макс 2,2 ₽/запрос), при обрезании ответа — код выхода 3.
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

## 7. Следующая итерация (предложение)

1. Разбить `spec-stage1.md` на два дробных ТЗ, прогнать через DeepSeek.
2. Перенести код в `bks-term/` workspace самостоятельно (не вслепую).
3. Прогнать fmt/clippy/test/release; выдать путь к `.exe` и чек-лист ручной проверки NVDA (раздел 8 плана разработки).
