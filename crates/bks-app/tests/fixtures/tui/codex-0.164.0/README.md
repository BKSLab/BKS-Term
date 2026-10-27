# Codex CLI 0.164.0: startup Folder access

Эталон относится только к официальному Codex CLI **0.164.0**. Это локальный
эвристический адаптер экрана, не структурированный App Server adapter.

- `official-folder-trust-sandbox-70x14.txt` — текст тела официального snapshot
  `renders_snapshot_for_git_repo`, без служебного YAML-заголовка. Источник:
  https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/tui/src/onboarding/snapshots/codex_tui__onboarding__trust_directory__tests__renders_snapshot_for_git_repo.snap
- `startup-folder-trust-70x14.txt` — тот же эталон с обычной строкой footer.
  Это вариант, выведенный из официального кода, **не запись живого запуска**:
  https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/tui/src/onboarding/directory_trust.rs
  (`show_windows_create_sandbox_hint: false`).
- Текст, пункты и защитная логика: https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/tui/src/onboarding/trust_directory.rs
- Фиксированные клавиши: https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/tui/src/onboarding/keys.rs

Проверено по официальному исходному коду: `1` выбирает первый пункт, затем
`Enter` подтверждает его; `2` сразу выходит независимо от выбора. Распознавание
само никогда не отправляет клавиши. Перед отправкой явного выбора проверяются
версия CLI, профиль, активная живая сессия, revision и весь текущий снимок.

Поддерживается только полный startup-диалог с `Trust and continue` / `Quit`.
Перенесённый или сокращённый путь, предупреждение о корне репозитория, ошибки
сохранения, restricted/resume/back-варианты и неизвестный текст отключают
распознавание. Оригинальный полный disclosure и footer доступны в native UI.

**Живой захват этого диалога не подтверждён.** Изолированный запуск с временными
настройками не дошёл до trust-экрана; stdin ему не передавался. Эти fixtures
основаны на версионированном официальном исходнике. Проверка реального диалога
и ручная NVDA-приёмка остаются отдельными пунктами приёмки.
