# BKS Term — память проекта (рабочее состояние)

**Обновлено:** 6 ноября 2026 г.
**Назначение:** снимок текущего состояния для продолжения работы в новых сессиях агента. Обновлять после каждой завершённой итерации. Концепция и план — в `docs/concept.md` и `docs/setup-and-iterations.md`; при противоречии те документы важнее.

## 1. Где мы находимся

### Контрольная точка — 06.11.2026, preview.34 — README и фиксация результата

Пользователь сообщил «вроде ок» после проверки последней сборки и явно
поручил commit в `main`, push и обновление README с новым дизайном и логотипом.
Это разрешение относится к накопленному результату данного проекта и
`origin/main` (`https://github.com/BKSLab/BKS-Term.git`); полная ручная приёмка
с NVDA этим сообщением не подтверждена.

README показывает круглый логотип Frosted Glass и настоящее окно preview.34
с демонстрационным диалогом. Снимок `docs/screenshots/bks-term-frosted-glass.png`
получен через PrintWindow собственного изолированного окна без редактирования
изображения; пользовательские окна и настройки не затрагивались. Все 12
проверок сценария снимка passed, собственные процессы закрыты. Квитанция:
`target/readme34/capture-receipt.json`.

В portable/Setup добавлены логотипы, снимок, паспорт, документация адаптеров
и исходный дизайн-референс. Скриншот актуального интерфейса отделён в README
от раннего референса Frosted Indigo. Обновлён CHANGELOG. Упаковка и проверка
установки/удаления в частных папках повторены: **48 passed**, 1 skipped
(file-symlink fixture, Windows error 1314). Реестр и ярлыки пользователя
проверкой не изменялись. Логи: `target/readme34/package{,-test}.log`.

Rust-код после полной проверки preview.34 не менялся: fmt, clippy с
`-D warnings`, **947 workspace tests** и release passed; актуальные логи
остаются в `target/codex-tools34/checks/`. SHA256 исполняемого файла сохранён:
`219386D902C8609BCD98CE59D01AC58B56402D0F5DB1BDEE55E16B44B3FCEBF6`.
Ограничения автоматической UIA-проверки и ручной приёмки NVDA ниже сохраняются.

В контрольную точку входят исходники, тестовые записи, ADR, документация
и ресурсы завершённых итераций. `.gitattributes` сохраняет исходные CR/LF
байты VT-записей; их протокольные пробелы не нормализуются. Локальная
`docs/note.txt` остаётся вне коммита; сборки и служебные логи — в `target/`.

### Итерация перед контрольной точкой — 06.11.2026, preview.34 — процесс Codex и финальный ответ

Пользователь сообщил: долго нет речи о процессе, а чтение результата начинает
с Ran/stdout вместо финального текста. Скриншоты просмотрены, в repo не копировались.
Пользовательского settings.json при проверке нет; default speech mode — Important.

Реализовано: новый `tui_turns_codex_activity.rs` отличает native exec от model
message по marker/title ink, dim gutter и геометрии, не по слову Ran одному.
Tool stdout не открывает fences и не создаёт фазы. Проvisional model headers
при текущем Busy не публикуются как финальный ответ и исключаются из следующего
полного snapshot; промежуточные Answer до последней activity также удаляются.
Обычный final, начинающийся Ran, сохраняется. Блоки `/status` preview.33 сохранены.
Clipped Esc footer доказывается стилями elapsed/hint, bold Esc, source editor
и правым краем, поэтому длинный видимый work title не теряет Busy.

`TurnLog::working_turn` даёт ID только для текущего source-owned Codex Busy
без menu/protected/terminal signal. `TurnSpeech::working` допускает короткое
«Работа продолжается» после 15 секунд без выданной речи, без очереди пропущенных
интервалов. Manual/background/typing/pause/terminal outcomes отменяют допуск.
Main обновляет его после событий; неполный sync frame временно задерживает
выдачу без сброса quiet timer. Ход работы не становится model Ready.

Проверена реальная isolated CLI 0.164 запись: два заранее фиксированных чтения
собственного файла (80 строк stdout), Sleep15s, commentary, два final messages,
включая обычную фразу Ran. 42 360 native VT bytes, 15 233 мс неизменного VT,
1437 реальных пустых polls в тихом интервале. Session replay получает heartbeat
внутри этого интервала, только final в answer body и два отдельных Ready.
Native Running label не наблюдался, его grammar проверяется synthetic tests.
Скрытые рассуждения не извлекались. Private readonly профили отклонили даже
safe spawn; положительный renderer test использует явно разрешённый собственный
test-only no-sandbox профиль, whitelist точных команд/cwd и loopback provider.
Это не проверка sandbox/approvals рабочего профиля; настройки пользователя не менялись.
Fixtures/README: `crates/bks-app/tests/fixtures/tui-codex-tools34/`.
Provenance/cleanup: `target/codex-tools34/validation-cleanup.json`, собственных процессов 0.

Класс M; доступные агенты Codex, один independent code-review. Review нашёл
два P2: Running stdout fence мешал footer scan; menu стирал provisional headers.
Оба воспроизведены ДО исправления и passed ПОСЛЕ: footer pass только защищает
стилизованные continuations, реальная роль Running всё ещё требует Busy;
menu/protected сохраняют прежнее доказательство commentary. Новый полный
review не запускался, адресная проверка закрыла оба замечания.

Проверки: 60 Codex regressions и 21 speech tests passed. После двух review
fixes повторены все mandatory: fmt, clippy workspace/all-targets `-D warnings`,
workspace tests, release — **passed**. **947 tests passed**, 0 failed/ignored:
app 859, worker 3, PTY 21, ConPTY 14, environment 2, terminal 48.
Логи: `target/codex-tools34/checks/`, initial —
`pre-review-*.log`, точные before/after — `review-{running,menu}-{before,after}.log`.
Добавлены 12 timed speech tests, 8 activity grammar tests, lifecycle и actual
Session workflow. Обновлены справка, manual, ограничения, CHANGELOG и ADR 0024.

Собрана **0.4.0-preview.34**, SHA256
`219386D902C8609BCD98CE59D01AC58B56402D0F5DB1BDEE55E16B44B3FCEBF6`.
EXE: `target/release/bks-app-0.4.0-preview.34.exe`.

Private native GUI: **31 passed**, 0 failed/error, 1 skipped. Строго проверены
настоящие tool calls/Sleep17/stdout, final-only journal, обычный Ran final,
native/UIA text, позиции чтения, фокус, отсутствие лишних POST и `/quit` → shell.
Автоматические UIA progress events честно skipped: registration=true, но
control delivery=false и foreground=false на неактивном private desktop;
gate не обходился, fake events нет. Это не подтверждение речи NVDA.
Логи: `target/codex-tools34/native-gui/runs/run-20261106-200821-04199b0d/`.
Cleanup receipt Passed=true, own processes 0, desktop/job/listener закрыты;
EXE hash совпал. Частная настройка CLI не затрагивает профили пользователя.

Portable/setup: `target/packages/bks-term-0.4.0-preview.34-windows-x64-*`.
Package/install/uninstall в Windows PowerShell 5.1: **48 passed**, 1 skipped
(file-symlink fixture, Windows 1314); пользовательская интеграция не менялась.
Итоговый diff/whitespace check passed; логи остаются в target.

Открыто пользовательское окно `.34` с `--profile cli-codex`, PID **13292**,
HWND **25561916**, Responding=true; ввода агент не отправлял. Квитанция:
`target/codex-tools34/launched-user-app.json`. Прежние пользовательские окна
не закрывались. Commit/push нет, прежние незакоммиченные правки сохранены.
**Дальше:** человек проверяет слышимость сообщений о ходе работы/напоминания
после 15 секунд и чтение только финального ответа в новом окне `.34` по
`docs/manual-final.md`. Агент не объявляет NVDA-проверку пройденной.

### Предыдущая точка остановки — 06.11.2026, preview.33 — сохранённые блоки вывода команд

Пользователь подтвердил чтение `/status` в preview.32, но уточнил требование:
результат команды должен оставаться блоком в общей истории после простого
набора, следующего запроса и перерисовки. Открытие текущего исходного экрана
не решает сохранение. Скриншоты и данные учётной записи в репозиторий не копировались.

**Реализовано:** `TurnLog` хранит Answer/Command с общими стабильными ID,
отдельными модельными и общими индексами. Ctrl+Up/Down проходят все сохранённые
блоки по порядку, Ctrl+Shift+R открывает последний новый блок. В общем документе
команда получает заголовок «Вывод команды», ответ модели сохраняет запрос.
Команда не создаёт model Ready/Phase, не раскрывается через request action
и не становится исполняемым текстом. Общий лимит — 128 блоков / 2 МиБ;
retention защищает активный ответ и новейший результат, обрезая старый текст.

Новый `tui_command_output.rs` сохраняет текст/оформление реального результата:
принятый Enter исходной slash-команды/подсказки, подтверждённое очищенное поле,
закрытое меню, exact plain echo и устойчивый 150 мс body до composer.
Повторный `/status` может не печатать новых slash-байтов и сохранять то же
тело: это отдельный snapshot результата принятой команды, не модельная новизна.
Для Tab требуется недавний подтверждённый выбор и точное фактическое поле с
кареткой перед Enter; учитывается добавленный Codex пробел. Новый ввод,
смена геометрии/буфера/адаптера, direct mode и clear сбрасывают незавершённый
захват. Результат команды ограничен 128 КиБ с отметкой об обрезке.
При неоднозначности остаётся исходное чтение. История не переносится из старых окон.

Изменены `tui_turns.rs`, `tui_transcript.rs`, `session.rs`, `main.rs`, меню и
справка; добавлены ADR 0023 и регрессии. Источник приёма ввода, вывод первого
символа и модельное распознавание сохранены. Класс M; реализация доступным
агентом Codex, один независимый code-review. Маршруты Kimi/K3 из скилла
не предоставлены текущими инструментами, работа не блокировалась.

**Проверки:** fmt, clippy workspace/all-targets `-D warnings`, workspace tests,
release — **passed**. **925 tests passed**, 0 failed/ignored: app 837,
worker 3, PTY 21, ConPTY 14, environment 2, terminal 48. Независимый review
существенных замечаний не выявил. Новые 12 pure tests проверяют admissions,
отрицательные случаи, Unicode/styles, Tab, expiry и отмены. Session replay
проходит пять настоящих сценариев из fixtures .32/.33: первый статус до
модели, typing, три command blocks/два ответа, одинаковые повторы, menu/empty
Enter, clear/reflow, search и место чтения. Отдельно проверены общие memory/count
лимиты при продолжающемся активном ответе. Логи: `target/codex-command33/checks/`.

Новые реальные captures: `crates/bks-app/tests/fixtures/tui-codex-command33/`,
53 912 исходных VT-байт с полными реальными polls и logical/wire ledger.
Локальный фиксированный SSE без пользователя/авторизации/внешней модели;
оба CLI естественно `/quit` с кодом 0, своих процессов 0. Picker использует
отдельный native Win32 Escape; его replay не объявляется проверкой literal
transport самого приложения. Provenance/cleanup — `target/codex-command33/`.

Собрана **0.4.0-preview.33**, SHA256
`D112FED085BE005C2C71AC44AE391214E2F5DDEA5F2A272B0FB47CAAA037C9DB`.
EXE: `target/release/bks-app-0.4.0-preview.33.exe`. Commit/push не выполнялись,
предыдущие незакоммиченные изменения сохранены.

Нативный GUI на private desktop: **50 passed**, 0 failed/error, 1 skipped
(фактическая каретка/NVDA недоступны в private desktop). Проверены первый
статус до модели, точные сохранённые тела трёх команд, пять отдельных позиций
чтения, Ctrl+Up/Down, поиск с Enter, Ctrl+Home, два точных модельных запроса
и `/quit` → OpenShell. EXE hash совпал; чужие окна/клавиши/clipboard не трогались.
Логи: `target/codex-command33/native-gui/runs/run-20261106-191002-cbf62d44/`.
Cleanup receipt: `target/codex-command33/native-gui/cleanup-verification.json`,
Passed=true, собственных процессов 0, desktop/job/listener закрыты.

Portable/setup: `target/packages/bks-term-0.4.0-preview.33-windows-x64-*`.
Упаковка, частная установка/удаление в Windows PowerShell 5.1: **48 passed**,
1 skipped (symlink fixture, Windows 1314); пользовательская интеграция не менялась.
Итоговый whitespace check passed. Служебные логи — только в `target/`.

Открыто новое пользовательское окно `.33` с `--profile cli-codex`, PID **22712**,
HWND **1967594**, Responding=true; ввода агент не отправлял. Окно `.32`, PID
**19848**, оставлено работающим. Квитанция: `target/codex-command33/launched-user-app.json`.
**Дальше:** человек проверяет `/status` → обычный запрос → Ctrl+Up/Down и речь
NVDA в новом окне `.33` по `docs/manual-final.md`. Старый журнал `.32` в новое
окно не переносится; агент не объявляет NVDA-проверку пройденной.

### Предыдущая точка остановки — 06.11.2026, preview.32 — чтение служебных команд Codex

Пользователь подтвердил исправление первого символа в preview.31. Новая
проблема на его скриншотах — результат `/status` виден на исходном экране,
но Ctrl+Shift+R открывает только прежний ответ модели. Скриншоты и сведения
учётной записи в репозиторий не переносились.

Исправлен выбор документа: после принятого Enter служебной команды, меню
или неизвестной формы Ctrl+Shift+R открывает исходный вывод. Новый запрос
выбирает архив после появления нового ID ответа; до этого остаётся доступен
исходный вывод. Вход в исходный документ явно снимает `answers_view`, закрывает
поиск и сбрасывает ключ VT-кеша. Ctrl+Up/Down сохраняют навигацию по ответам.
Политика обновляется только после успешной передачи явного ввода; команды
из текста вывода не исполняются. Оболочка и принятие echo первого символа
не менялись.

Изменения: `LatestOutput` в `tui_transcript.rs`, методы сессии в `session.rs`,
маршрут Ctrl+Shift+R и принятый ввод в `main.rs`. Общий `ReplyAnchor` сохранён:
при неоднозначной перерисовке чтение начинается с текущей страницы. Вывод
служебных команд остаётся в исходном буфере/истории терминала, не становится
телом ответа модели и может быть удалён самим CLI при очистке или перерисовке.
Уточнены справка, ручная приёмка, ограничения и ADR 0015.

Добавлены три настоящие изолированные записи Codex 0.164.0 с фиксированным
loopback SSE: `/status` + Enter, `/st` + Enter по выбранной подсказке,
`/st` + Tab + Enter. В каждой — два запроса и ответа, локальный статус между
ними, точные logical/wire bytes, все реальные polls и естественный `/quit`
с кодом 0. Служебная команда не добавляет POST модели. Fixtures:
`crates/bks-app/tests/fixtures/tui-codex-command32/`; собственные процессы
capture завершены. Session replay проверяет source/answers выбор, выход из
ранее открытого архива, поиск статуса и сохранность двух ответов. В этих
коротких captures первая запись может остаться незавершённой до следующего
запроса: завершение провайдера само по себе не подменяет source completion.

**Проверки:** fmt, clippy workspace/all-targets `-D warnings`, workspace tests,
release — **passed**. **911 tests passed**, 0 failed/ignored: app 823,
worker 3, PTY 21, ConPTY 14, environment 2, terminal 48. Независимый узкий
проход code-review не выявил существенных замечаний; diff и whitespace
check passed. Логи: `target/codex-command32/checks/`.

Собрана **0.4.0-preview.32**, SHA256
`9A96E160EA0800548968AEFED6424C4AAB400B2B46C54F0FEDB9473B10813321`.
EXE: `target/release/bks-app-0.4.0-preview.32.exe`; portable/setup:
`target/packages/bks-term-0.4.0-preview.32-windows-x64-*`.
Упаковка и частная установка/удаление: **48 passed**, 1 skipped
(file-symlink fixture, Windows 1314). При первом запуске из PowerShell Core
установщик получил неверный унаследованный PSModulePath и не нашёл Get-FileHash;
повтор в штатном Windows PowerShell 5.1 с его module paths — passed.
Установочные скрипты не менялись, пользовательская интеграция не затрагивалась.
Нативный GUI на private desktop: **24 passed**, 0 failed/error, 1 skipped
(реальная каретка/NVDA недоступны на private desktop). Проверены source-текст
RichEdit 1018 и UIA, поиск `Token usage` с Enter, Ctrl+Up к прежнему ответу,
Ctrl+Shift+R обратно к статусу из архива, второй точный запрос/новый ответ
и `/quit` с OpenShell. Первая попытка harness ошибочно ожидала выделение
до Enter в поиске; исправлен только target-helper, итоговый прогон passed.
Логи: `target/codex-command32/native-gui/runs/run-20261106-182334-d38d16ac/`.
Собственные процессы, job и private desktop закрыты, EXE hash совпал.
Commit/push не выполнялись, прежние незакоммиченные изменения сохранены.

Открыто новое пользовательское окно `.32` с `--profile cli-codex`, PID **19848**,
HWND **10881382**, Responding=true. Ввод в него агент не отправлял;
пользовательские окна не закрывал. Квитанция:
`target/codex-command32/launched-user-app.json`.
**Дальше:** человек проверяет чтение служебных команд и фактическую речь NVDA
в новом окне по `docs/manual-final.md`. Агент не объявляет NVDA-тест пройденным.

### Предыдущая точка остановки — 06.11.2026, после preview.31 — первый символ TUI

Пользователь в целом принял работу Codex, но сообщил о пропуске озвучивания
первой буквы TUI; в обычной оболочке она слышна. Ошибка воспроизведена на
настоящих paced/fast VT-записях: generic echo считал первую букву обычной
вставкой, тогда как она заменяет непустую экранную подсказку пустого поля.
Смена истории при первой перерисовке Codex также обрывала pending echo.

Исправлены `tui_input_echo.rs` и вызовы в `main.rs`: свежая подтверждённая
пустая форма исключает hint только из приватной baseline. Речь по-прежнему
требует реально видимых glyphs и каретки; при reflow Codex используется
текущая source projection. Перед arm обновляется состояние composer. Скрытый,
маскированный, невидимый или protected ввод, истечение срока, переключение
буфера и неизвестный hint не становятся предсказанной речью. Оболочка не менялась.

**Проверки:** fmt, clippy workspace/all-targets `-D warnings`, workspace tests,
release — **passed**. **908 tests passed**, 0 failed/ignored: app 820,
worker 3, PTY 21, ConPTY 14, environment 2, terminal 48. Новая регрессия
сравнивает весь озвучиваемый ввод на реальных fast/paced captures, включая
первую букву и отсутствие повторов; отдельно проверены masking/concealment,
защищённые метаданные, неизвестная подсказка, buffer switch и expiry.
Логи: `target/first-input-echo31/checks/`. Diff просмотрен, whitespace check passed.

Собрана **0.4.0-preview.31**, SHA256
`58EFFADE17E85513D6E33C44DE3D40617378DF7A10F42E7007B497A0486809F3`.
EXE: `target/release/bks-app-0.4.0-preview.31.exe`; portable/setup —
`target/packages/bks-term-0.4.0-preview.31-windows-x64-*`.
Упаковка и частная установка/удаление: **48 passed**, 1 skipped
(file-symlink fixture, Windows 1314); пользовательская интеграция не менялась.

По ранее разрешённому запуску открыто новое окно `.31` с `--profile cli-codex`,
PID **20540**, HWND **3082980**, Responding=true. Ввод агент не отправлял.
Прежнее пользовательское окно `.30`, PID **19508**, сохранено. Commit/push нет.
**Дальше:** пользователь проверяет фактическое озвучивание первой буквы с NVDA
в новом окне по `docs/manual-final.md`. Агент не объявляет NVDA-проверку пройденной.

### Предыдущая точка остановки — 06.11.2026, после preview.30 — интеграция Codex

Пользователь выбрал Codex следующим приоритетом; отдельную работу над OpenCode
пока отложил, Kimi остаётся ориентиром качества. Доработан нативный Codex CLI
**0.164.0**, собрана и упакована **0.4.0-preview.30**. Это release-кандидат:
человеческая NVDA/clean-VM приёмка не выполнена. Commit/push не выполнялись,
прежние незакоммиченные изменения сохранены.

**Что изменено:**

- `codex.rs` — отдельная политика, Win32 input records и проверка раскладки.
  Режим 9001 отслеживается VT parser в `bks-terminal`, включая split/reset,
  строковые payload и synchronized frames. Для выбранного Codex после запроса
  этого режима BMP literal передаётся VK_PACKET, supplementary scalars — UTF-8.
  Модификаторы Enter сохраняются; Shift+Enter реально добавляет строку.
  Протокольные ответы терминала не проходят через literal encoder.
- Первая полная вставка до 32768 UTF-16 получает bounded source proof; большая
  вставка Codex имеет срок 15 секунд, обычный ввод/удаления — 750 мс. Публичное
  поле содержит только actual source window. Приватное намерение полного
  текста не озвучивается как поле; полный новый echo после Enter подтверждает
  запрос для архива. Поддержаны переносы, startup repaint с ростом истории,
  resize и обрезанное по высоте поле. Его имя — «видимая часть ввода».
- Исправлен общий приоритет source proof: промежуточный неизменённый кадр
  обновлял revision панели и мешал подтверждению следующей вставки. Успешный
  recovery от исходного accepted origin теперь сильнее обычной palette
  projection. Это проверено и прежним реальным OpenCode resize capture;
  сам OpenCode не получал новых функций. Replay helpers используют согласованное
  время и каждый полный sync frame, без искусственного parser timeout.
- Tab во время ответа регистрирует Queue, Enter — Steer. TurnLog выбирает
  запрос по полному свежему echo, поэтому steering обгоняет очередь и одинаковые
  запросы остаются отдельными. Прерванный ответ сохраняется incomplete без Ready;
  исчезновение Working при первом токене не означает завершение. Preview
  queue/steering не попадает в ответы и речь. Полная identity Codex не обрезается
  до 8К лимита уведомления; prefix не подтверждает более длинный запрос.
- Управляемый профиль проверенной версии получает только
  `-c tui.disable_paste_burst=true`. Мигрирует лишь точный неизменённый старый
  шаблон после сверки файла/версии. Custom args/name/cwd/settings и KeepTab
  сохраняются; глобальный Codex config не меняется. `--no-alt-screen` не добавлен.
- ADR: `docs/decisions/0022-codex-native-input-and-turns.md`; обновлены справка,
  описание адаптера, ограничения и ручная приёмка.

**Проверки:**

- `cargo fmt --all -- --check`, clippy workspace/all-targets `-D warnings`,
  `cargo test --workspace`, `cargo build --release` — **passed**.
  **903 tests passed**, 0 failed/ignored: app 815, worker 3, PTY 21, ConPTY 14,
  environment 2, terminal 48. Логи: `target/codex-production30/checks/`.
- Настоящие изолированные Codex ConPTY captures и фиксированный loopback
  Responses SSE без пользовательских ключей/внешней модели/tool calls:
  `crates/bks-app/tests/fixtures/tui-codex-production30/`. Реальные Session/Submission/TurnLog
  regressions покрывают Unicode, 80/400/4К, newline, resize/menu, очередь,
  steering, повторные запросы и 75-параграфный ответ. Отдельный adversarial
  source-equivalent тест проверяет полную identity >8К и отказ по одному prefix.
- Финальный release EXE проверен настоящим GUI и Codex: **13 passed**,
  0 failed/error, 1 skipped (UIA caret/range на private desktop; native EM_GETSEL
  и UIA Text проверены строго). `Ж界🙂 []`: caret 7; Shift+Enter: CRLF/caret 9;
  вторая строка: caret 23. До Enter conversation counts `[0,0,0]`; provider
  получил точный multiline запрос, ответ полностью появился в 1018, `/quit`
  вернул пустую editable OpenShell. Receipt:
  `target/codex-production30/final-native-gui-verification.json`;
  run `native-gui/runs/run-20261106-161525-60dc7cda/`.
  App/bootstrap exit 0, private desktop/job закрыты, собственных BKS/Codex 0.
- Portable и Setup: **48 package/install/uninstall checks passed** с реальными
  артефактами и частной установкой без интеграции/запуска приложения.
  **1 skipped**: file-symlink fixture, Windows 1314 (нет нужной привилегии).
  Никаких записей пользовательского реестра/ярлыков. Логи package*.log.
- Независимое ревью: закрыты P2 сохранения partial intent при resize и сброса
  private hint при смене primary/alternate/новом пустом composer. Регрессии passed.

**Существенное ограничение:** Windows ConPTY воспроизводимо искажает cursor
correction после некоторых combining marks: typed `Ж界🙂 é []` доходит до
провайдера точно 10 UTF-16, но VT/нижнее поле временно содержат 9 без пробела.
Полный новый echo после Enter уже содержит 10; приватный literal hint позволяет
сохранить правильный полный запрос только после этого подтверждения. Физическая
каретка/нижний текст не угадываются. Для ZWSP/части ZWJ sequences без полного
faithful echo остаётся обычное чтение source, без выдуманного архива/Ready.
Прямое окно и `--no-alt-screen` одинаково воспроизводят этот upstream gap.
См. `docs/known-limitations.md` и отдельные positive/negative capture contracts.

**Release:** `target/release/bks-app.exe` и
`target/release/bks-app-0.4.0-preview.30.exe` совпадают: **3477504** байт,
SHA256 `D549CD4FA64CC190AF7D8F95CA6BE6B756921021FBADBF0896741B0B06DC281B`.
Portable ZIP, Setup EXE, manifests, dependencies и SHA256SUMS —
`target/packages/bks-term-0.4.0-preview.30-windows-x64-*`.

**Дальше:** ручная приёмка Codex с NVDA/физической клавиатурой и рабочей
авторизацией по `docs/manual-final.md`, затем чистая Windows VM. Автопроверки
не считаются подтверждением речи NVDA или окончательного production release.
В пользовательские окна ввод не отправлялся; пользовательский Codex не запускался.

### Предыдущая точка остановки — 05.11.2026, после preview.29 — первый ввод OpenCode

Исправлена жалоба пользователя: сразу после появления формы OpenCode ввод
оставался в исходном поле наверху, а общее поле внизу было пустым. Собрана и
запущена **0.4.0-preview.29**. Commit/push не выполнялись; прежние изменения
сохранены.

**Причины и исправления:**

- В начале OpenCode показывает textarea раньше полного footer. Первый символ
  мог быть принят при `composer=None`; после этого строгий recognizer уже не
  подхватывал непустое поле. Теперь только для выбранного OpenCode полная
  геометрия, новый source revision, начальные координаты принятого ввода и
  видимая каретка подтверждают точный префикс явно принятых символов. Без
  прежнего поля сохраняется лимит 64 символа; неизвестный prefill не переносится.
- Отдельно воспроизведена потеря поля после resize. При явном resize частная
  baseline сохраняется до двух секунд; новая исходная перерисовка должна
  подтвердить весь канонический текст, буфер, режим и палитру. Старая копия
  сетки после `TerminalModel::resize` сама не подтверждает reflow.
- Принятое новое нажатие отменяет resize-baseline. Независимый review выявил
  потерю Shift+Enter сразу после восстановления; исправлено и проверено
  реальной фазой resize → newline → Backspace, до нового печатного символа.
- Временная числовая трассировка удалена. Ни одна правка не отправляет
  автоматически распознанные из TUI команды или подтверждения.

**Проверки:**

- Установленный OpenCode **2.0.32**, настоящие изолированные ConPTY-записи
  раннего ввода и resize, без реальных LLM/tool запросов; штатный `/exit`, code0.
  Fixtures: `crates/bks-app/tests/fixtures/tui-opencode-input29/`; полные
  captures: `target/opencode-input29/captures/{first-source,resize-newline}/`.
- Первый ввод воспроизведён на `.28` трижды и на optimized trace `.29` до
  исправления. Debug-сборка меняла время запуска и скрывала этот race;
  окончательная GUI-проверка использовала обычную release-сборку.
- `cargo fmt --all -- --check`, clippy workspace all-targets `-D warnings`,
  `cargo test --workspace`, `cargo build --release` — **passed**.
  **865 tests passed**, 0 failed: app780, worker3, PTY21, ConPTY14,
  environment2, terminal45. Логи: `target/opencode-input29/checks/`.
- Три actual GUI-сценария release `.29` + настоящий OpenCode — **passed**:
  два немедленных первых ввода (450/100 мс между буквами), resize
  `100×29 → 124×35 → 100×29` и последний символ после возврата. На каждом шаге
  нативное нижнее поле содержит точный текст и UTF-16 caret, верхнее исключает
  активную форму. Summary: `target/opencode-input29/final-gui-regression29.json`.
  Private desktops и собственные процессы проверки закрыты.
- Целевой независимый review нового first-input подтверждения не нашёл
  значимых дефектов; P2 resize/newline закрыт. Diff/whitespace checks passed.

**Release и запуск:** `target/release/bks-app.exe` и
`target/release/bks-app-0.4.0-preview.29.exe` совпадают: **3 418 624** байт,
SHA256 `E9A7EDA0165C7FAD0C9A9EE9F14B5CC173055E508B244BE39A4EF133B4DF8C85`.
05.11.2026 15:50:58 +04 запущен основной EXE с `--profile cli-opencode`,
PID **21680**, HWND **13959744**, Responding=true; подтверждён настоящий
дочерний `opencode.exe --standalone`, PID **6368**. Начальный discovery
`--version` не считается запущенным пользовательским CLI. В новый живой
пользовательский CLI агент ввод не отправлял. Прежний основной образ `.28`
сохранён в `target/release/bks-app-active-preview.28.exe`; пользовательские
окна/процессы агент не завершал.

**Дальше:** пользователь проверяет первый ввод в открытой `.29`, затем
остальные сценарии отдельного OpenCode-профиля. NVDA, физическая клавиатура
и авторизованные провайдеры остаются человеческой приёмкой.

### Предыдущая точка остановки — 05.11.2026, после preview.28 — отдельный OpenCode

Пользователь принял сворачиваемые запросы и прежнюю работу Kimi. По поручению
проверен код отдельного OpenCode-профиля и доработан до тех же сценариев ввода,
чтения и навигации. Собрана **0.4.0-preview.28**; основной EXE и копия
`target/release/bks-app-0.4.0-preview.28.exe` совпадают. Commit/push не выполнялись,
все прежние незакоммиченные изменения сохранены.

**Что исправлено:**

- `opencode.rs` — отдельный модуль политики, явных клавиш и defaults профиля.
  Реальное `opencode v2.0.32` теперь распознаётся. Ограниченный help probe
  подтверждает `--standalone` перед добавлением аргумента; неизвестная версия/
  возможность сохраняет прежний обычный запуск. Встроенный профиль использует
  приватный сервер клиента и процессный полный показ вставки. Глобальная служба
  и файлы конфигурации не перенастраиваются; существующая inline-переменная
  `OPENCODE_CLI_CONFIG_CONTENT` имеет приоритет. Сохранённые custom profiles
  не переписываются, прежний OpenShell после CLI сохранён.
- Shift/Ctrl/Alt+Enter передаются CSI-u с исходными модификаторами вместо CR,
  который ошибочно отправлял запрос. Обычный Enter прежний; остальные CLI и
  прочие клавиши используют прежний encoder. Решение о действии остаётся у CLI.
- Первая полная bracketed paste длиннее 64 символов подтверждается до 32K
  только в выбранном OpenCode с доказанным исходным полем. Предел UTF-16,
  protected/source/geometry/style/caret guards и 750 мс сохранены. Исправлен
  ложный WRAPLINE фоновой заливки. Собственный word wrap renderer сверяется
  со всем явно принятым текстом, не добавляя экранные переносы в запрос.
- Backspace/Delete после такого wrap сохраняют канонические текст и каретку:
  проверяется фактическое однозначное удаление, а не предполагаемое число
  UTF-16/scalar/grapheme единиц. Смена высоты поля допустима только в проверенном
  OpenCode; source, columns, palette и результат удаления остаются обязательными.
- Ctrl+X → Enter сохраняет краткое намерение очереди (3 секунды) и документ
  до скрывающей поле leader-перерисовки. Текст редактора больше не считается
  предыдущим semantic echo. Меню, защита, смена текста, другая клавиша и timeout
  отменяют намерение; новый полный source echo по-прежнему обязателен для
  разделения ответов. Оба запроса/тела/ID и disclosure доступны. Без стабильной
  idle-границы первый queued-ответ остаётся incomplete, последний получает
  собственный Ready; ожидания обычных двух ответов не ослаблялись.
- Shell определяется только в исходном toolbar-слоте. `!` и Escape не портят
  последующее подтверждение текста, shell-черновик читается и не становится
  запросом к модели. Никакие команды/разрешения из вывода не выполняются.

**Проверки и происхождение:**

- Установлен OpenCode **2.0.32**. Повторно прочитаны официальные TUI/settings/
  keybinds/standalone docs; детали и ссылки в `docs/cli-adapters.md#opencode`.
- Actual owned ConPTY, private home/config/data/cwd, запрет tools/MCP/plugins,
  без user auth и настоящей LLM-сети. Editor run: ноль HTTP/model/tool запросов,
  Unicode/новые и пустые строки, большая первая вставка, wrap, Backspace/Left/
  Delete, Shell/Escape и natural `/exit` code 0. Queue run: два фиксированных
  loopback SSE ответа, штатная очередь во время stream, natural `/exit` code 0.
  Canonical raw и временные ledgers —
  `crates/bks-app/tests/fixtures/tui-opencode-profile28/`; полный provenance —
  `target/opencode-profile28/captures/{editor-full-profile3,queued-default}/`.
  Ранний editor-default — диагностический запуск, не успешная приёмка выхода.
- `cargo fmt --all -- --check`, clippy workspace all-targets `-D warnings`,
  `cargo test --workspace`, `cargo build --release` — **passed**.
  Тесты **860 passed**, 0 failed: app775, worker3, PTY21, ConPTY14,
  environment2, terminal45. RUST_TEST_THREADS=4. Семь actual Session replays
  включают прежние Kimi/Codex/Claude/OpenCode и новые редактор/очередь OpenCode;
  точные UTF-16 caret проверены на каждой редакторной фазе. Логи:
  `target/opencode-profile28/checks/{fmt,clippy,tests,release}.log`.
- GUI SingleInput: **211 passed / 29 skipped / 0 failed / 0 error**,
  `target/opencode-profile28/gui-single/run-20261105-113342-e513d3fc/summary.json`.
  GUI TuiTurns: **259 passed / 4 skipped / 0 failed / 0 error**,
  `target/opencode-profile28/gui-turns/run-20261105-113342-7caa1d02/summary.json`.
  Итого **470 passed / 33 skipped**; оба GUI hashes совпадают с release.
  Skips — private-desktop foreground/interactive UIA и человеческая NVDA.
- Один независимый code-review выявил P2: потеря канонизации после удаления
  в wrapped input. Исправлено, добавлены настоящие Backspace/Left/Delete
  фазы и отрицательные проверки; последующие fmt/clippy/860tests/release passed.
  Нового полного круга ревью не было. Статический diff --check passed.
- Release: **3 411 456** байт, SHA256
  `8B5C55F23A00D8E599C3BB0A05E01547575E9FE3069D75A4149DBD250343B3A3`.

**Запуск:** 05.11.2026 11:36:27 +04, PID **20548**, HWND **394260**,
Responding=true. Запущен основной EXE с `--profile cli-opencode`; подтверждён
настоящий дочерний `opencode.exe` PID **21252**, ParentPID20548,
`--standalone=true`. В пользовательский CLI запросы агент не отправлял.
Пользовательские окна/процессы агент не завершал.

**Дальше:** ручная проверка нового OpenCode-профиля: ввод/многострочная вставка,
меню и варианты моделей, очередь, чтение/раскрытие запросов, Ctrl+Up/Down,
выход `/exit` и новая команда в оболочке. Речь NVDA, физические клавиши и
реальные авторизованные провайдеры остаются человеческой приёмкой. Новых
известных незавершённых правок кода нет.

### Предыдущая точка остановки — 05.11.2026, после preview.27

Пользователь подтвердил: по Kimi вопросов нет. По согласованному предложению
добавлены сворачиваемые отправленные запросы перед ответами в журнале чтения.
Собрана и запущена **0.4.0-preview.27**: `target/release/bks-app.exe`, копия
`target/release/bks-app-0.4.0-preview.27.exe`. Commit/push не выполнялись;
прежние незакоммиченные изменения сохранены.

**Поведение:** Ctrl+Shift+R открывает последний ответ; выше находится жирный
заголовок «Ваш запрос, свёрнуто» с началом сообщения (до 80 символов).
Enter/пробел при каретке на заголовке раскрывают полный текст, повторное
нажатие сворачивает. Текст можно выделять и копировать. Выделенный заголовок,
тело запроса и ответ не являются действием; повторы удержанной клавиши
не переключают блок. Состояние указано в тексте и объявляется при переключении.
Ctrl+Up/Down по-прежнему ставят каретку на начало ответа; автоматическое
чтение берёт только его тело. Поле ввода и PTY при раскрытии не изменяются.

`tui_turns.rs` сохраняет отдельный полный request рядом с answer, независимо
от прежнего ограниченного input для семантического доказательства. Исходные
правила подтверждения отправки, menus/protected/slash и завершения ответов
сохранены. RequestTarget использует канонические UTF-16 позиции; клавиатурное
действие требует актуальной ревизии опубликованного нативного документа.
CR/LF нормализуются для RichEdit без потери пустых строк и Unicode.
Общий лимит 128 пар/2 МиБ учитывает также скрытый запрос; отдельный запрос
ограничен 128 КиБ с явной отметкой о несохранённом конце. Раскрытие хранится
в своей записи и переживает обновления/смену вкладки/адаптера. Архив по-прежнему
только в памяти вкладки: старые запросы из прежнего процесса не восстанавливаются.

**Проверки:**

- `cargo fmt --all -- --check` — passed.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed.
- `cargo test --workspace` — **848 passed**, 0 failed: app763, worker3,
  PTY21, ConPTY14, environment2, terminal45; RUST_TEST_THREADS=4.
  Семь новых request-regressions: Unicode/multiline, выделение и ложные
  заголовки, offsets/styles/navigation, длинный ввод, limits/retention,
  bookmark при новом ответе. Все пять actual Session-replays Codex/Claude/
  OpenCode/Kimi ordinary/queued дополнены проверкой точного сохранённого
  запроса, независимых блоков и отсутствия новых PTY writes/events.
- `cargo build --release` — passed. Размер **3 397 120** байт, SHA256
  `317C984F31316D5E19FBC8E9C0A8AA8C09401F80E62F249C9E002E00611A34BF`.
  Логи проверок: `target/request-reader27/{fmt,clippy,tests,release}.log`.
- GUI TuiTurns — **259 passed / 4 skipped / 0 failed / 0 error**:
  `target/request-reader27/gui-turns/run-20261105-093408-362b11de/summary.json`.
  Enter/Space/held repeats, selected heading/body negatives, readonly/UIA,
  фокус, два независимых блока, navigation и ноль PTY bytes — passed.
  Skips: два foreground-зависимых focus-перехода на private desktop и две
  группы интерактивной/человеческой приёмки. NVDA и clipboard не имитировались.
  Снимок `tui-request-expanded.png` просмотрен: раскрытый первый и свёрнутый
  второй запрос читаемы, исходное оформление ответов сохранено.
- Один независимый read-only проход code-review — без значимых подтверждённых
  замечаний. Статический diff --check passed. Документация/справка обновлены.

**Запуск:** 05.11.2026 09:35:52 +04, PID **18280**, HWND **1378802**,
Responding=true. SHA release совпадает с GUI observations и versioned copy.
Прежний PID6328 продолжает работу; его EXE сохранён под именем
`target/release/bks-app-running-preview.26-6328.exe`, пользовательские сессии
не завершались. Проверка WaitForInputIdle вернулась до появления HWND;
последующий опрос подтвердил созданное окно, повторный процесс не запускался.

**Дальше:** пользователь проверяет в новом окне отправку многострочного запроса,
Ctrl+Shift+R → вверх к заголовку → Enter/пробел, чтение/выделение/копирование
и речь NVDA о состоянии. Физическая клавиатура и NVDA остаются ручной приёмкой.
Новых известных незавершённых правок кода нет.

### Предыдущая точка остановки — 02.11.2026, после preview.26

Реализованы исправления после жалобы Kimi: нет переходов между ответами,
после выхода недоступна оболочка. По поручению пользователя повторно проверены
все пять заявленных CLI. Собрана и запущена **0.4.0-preview.26**:
`target/release/bks-app.exe`, копия `target/release/bks-app-0.4.0-preview.26.exe`.
Commit/push не выполнялись; прежние незакоммиченные изменения сохранены.

Пользователь уточнил: Kimi запускался **отдельным профилем из меню**.
Точный текст/код ошибки дополнительно запрошен, ответа пока нет. В изолированном
прямом профиле выход Kimi даёт natural code 0; KeepTab оставлял завершённую
вкладку по прежнему default. Конкретный пользовательский OS crash не воспроизведён;
не объявлять его причиной найденное отдельно PS5-зависание.

**Что изменено:**

- Kimi 2.4.1: распознаётся обычный plain ● после принятого ❯, сохраняются два
  ответа и уникальные ID; работают latest/previous/next. Source-styled Working
  над full-width rounded composer даёт границу работы; software inverse caret
  нормализуется тем же reading_cursor, что используется projector. Реальный
  Session-тест поймал несовпадение raw/normalized frame после lowlevel pass;
  guard исправлен, complete-ожидания не ослаблены.
- Проверен повтор одинакового запроса во время stream. Оба ответа доступны,
  первый остаётся incomplete, если нет самостоятельной стабильной idle-границы;
  второй complete. Queue hint остаётся исходным control text, не телом ответа.
  Actual captures и source/styles/ACK/negative guards —
  `tests/fixtures/tui-kimi-2.4.1-two-answers/`.
- Встроенные CLI-шаблоны всех пяти tools, специальный direct Codex и новые
  пользовательские варианты теперь выбирают OpenShell по умолчанию. После CLI
  открывается свежая выбранная оболочка с журналом. Сохранённый пользовательский
  KeepTab не переписывается; serde/schema defaults и обычный exit оболочки прежние.
  Изменены только factories cli_profiles/codex_cli и New в launch_profile_editor.
- Отдельно воспроизведён настоящий Kimi → ответ → /exit → PS5 в длинном cwd:
  PowerShell жива, но ConPTY wrap меняет linear prompt fingerprint (CRLF,
  повтор граничной ячейки, отсутствует trailing space), интерфейс остаётся в TUI.
  Стандартные PowerShell 5/7 получают runtime-only process identity в полной
  паре OSC 133;A/B;BKS=<16hex>. Чужие/legacy/одиночные markers не дают handoff;
  пользовательские startup scripts и сохранённые args не меняются. Native
  before failed / after passed + первая новая команда. ADR 0021 описывает решение.
  Crash recovery по-прежнему только standard pwsh + exact CLR0x80131623 +
  ownership/full drain; команды и старые клавиши не повторяются.
- Claude 2.1.240: выбранный callback claude_chrome.rs исключает right effort и
  muted elapsed-строки из архива/речи ответа только при подтверждённых source
  geometry/style. Quotes/code/ordinary ink/другой slot/неизвестный текст сохраняются.
  Elapsed сам по себе не завершает ответ. Codex/OpenCode semanticfix не требовался:
  initial endpoint-only findings исчезли при настоящем chunk/poll pipeline.
- Новый session_cli_replay_tests.rs: пять actual Session-regressions для
  Codex/Claude/OpenCode/Kimi ordinary/queued через source ACK, composer/history,
  menu, Submission, archive, Ready, text/styles и navigation. .gitattributes
  запрещает преобразование CR/LF для .vt, сохраняя byte offsets/hash fixtures.

**Повторная матрица:**

Codex 0.164.0, Claude 2.1.240 и OpenCode 2.0.32: реальные Unicode editors/menu,
два фиксированных ответа от собственных Responses/Anthropic/OpenAI-compatible
loopback providers, два Ready/ID, previous/next, natural /quit|/exit code 0.
Kimi 2.4.1: два ответа, queued repeat, source/Session/navigation и /exit;
KeepTab code 0, OpenShell → новая оболочка → новая команда passed.
PowerShell 5 long-cwd и PowerShell 7 interactive return также passed.
Gemini отсутствует в 29 discovery/PATH directories: installed smoke skipped,
существующий bounded fallback проверен. Установка/обновление CLI, user auth/config,
реальная LLM-сеть и пользовательские окна не использовались. Только own empty
workspaces/private per-process trust; никакие UI trust/actions по тексту не
подтверждались. Own PTY/listeners закрыты, exact-path cleanup remaining 0.
Матрица/provenance — docs/cli-adapters.md и target/cli-audit26/others/matrix.md.

**Итоговые проверки:**

- fmt --all --check, clippy --workspace --all-targets -D warnings — passed.
- cargo test --workspace — **841 passed**, 0 failed: app756, worker3, PTY21,
  ConPTY14, environment2, terminal45. RUST_TEST_THREADS=4. Логи:
  `target/cli-audit26/{fmt,clippy,tests,release}.log`.
- cargo build --release — passed; **3 388 416** байт, SHA256
  `95A50A486B63E652FBB6376E8145DCE991D284883508116AD06CF73AAC05156D`.
  Метаданные — target/cli-audit26/release.json.
- GUI SingleInput — **211 passed / 29 skipped / 0 failed / 0 error**:
  `target/cli-audit26/gui-single/run-20261102-223850-a73bdc0a/summary.json`.
- GUI TuiTurns — **216 passed / 3 skipped / 0 failed / 0 error**:
  `target/cli-audit26/gui-turns/run-20261102-223850-ced11bca/summary.json`.
  Итого **427 passed / 32 skipped**; обе manifest SHA совпадают с release.
  Skips — foreground/interactive UIA на private desktop и человеческая NVDA,
  не подтверждение доступности. Только own fixture-процессы закрывались.
- Адресно: owned prompt12, Session64, actual CLI Session5, turns55,
  Claude chrome3 passed. Единственный независимый read-only review завершён
  без значимых подтверждённых замечаний; новые defaults также просмотрены.
- Статический diff --check passed. После итоговой сборки изменялись только docs.

**Запуск:** 02.11.2026 22:40:21 +04, PID **24000**, HWND **12716900**,
Responding=true. target/cli-audit26/launch.json. Прежние пользовательские окна
агент не завершал. Проверка hash сделана для обоих GUI и versioned copy.

**Дальше:** пользователь проверяет свой Kimi-профиль, два ответа/переключение,
выход и новую команду оболочки. Если у сохранённого пользовательского варианта
выбрано KeepTab, действие меняется в редакторе вручную; built-in уже OpenShell.
Если сообщит настоящий код ошибки, сопоставить с этим выходом, не считать его
объяснённым одним PS5 wrap. NVDA, физическую клавиатуру/foreground и реальные
авторизованные сервисы принимает человек. Не объявлять все режимы CLI или
живой Gemini проверенными. Новых известных незавершённых правок кода нет.

### Предыдущая точка остановки — 02.11.2026, после preview.25

Реализован согласованный переход на профили CLI. Собрана и запущена
**0.4.0-preview.25**. Основной файл — `target/release/bks-app.exe`, копия —
`target/release/bks-app-0.4.0-preview.25.exe`. Работа сохранена в рабочем дереве,
без commit/push. Чужие и прежние незакоммиченные изменения сохранены.

**Новый подход:** общее ConPTY/VT/native ядро и отдельные стратегии Codex,
Claude, OpenCode, Kimi, Gemini; Universal для остальных TUI. Адаптер выбирается
независимо от команды. Несколько вариантов одного CLI могут иметь разные
аргументы, папку, переменные среды, оболочку и действие после выхода.
Официальная документация всех пяти CLI изучена; ссылки и границы подтверждения
находятся в `docs/cli-adapters.md`, архитектура — ADR 0020.

- `launch_profile.rs`: план прямого запуска либо запуска через выбранную
  PowerShell/cmd/Git Bash, общая оболочка и переопределение в профиле.
  Аргументы передаются буквально. CMD принимает только гарантированно
  представимые значения; сложные/пустые аргументы требуют другого способа.
  Скрипт можно запускать явным интерпретатором с отдельными аргументами.
- `launch_profile_editor.rs`: нативные EDIT/COMBOBOX/LISTBOX, страницы общих
  параметров, запуска, аргументов и среды; добавление/копирование/удаление
  пользовательских профилей. Встроенные шаблоны копируются для настройки.
  Вход: «Настройки → Профили и оболочка…» либо меню профилей →
  «Профили запуска и оболочка…». Сохранение применяется при следующем запуске,
  не исполняет команды и восстанавливает черновик при ошибке записи.
- `profile.rs`: чтение schema 1/2, сохранение schema 2, lock и трёхстороннее
  merge-save с сохранением чужих SSH/Local изменений и отказом при конфликте
  одного профиля. Коллизии новых Local ID разрешаются с обновлением ссылок.
- `terminal_environment.rs`: private environment дочернего процесса,
  Windows case-insensitive overlay, сохранение native UTF-16/hidden entries.
  Родитель и другие вкладки не меняются, значения не выводятся в логи.
- `session.rs`: после полного drain CLI можно оставить завершённую вкладку
  либо один раз открыть новую выбранную оболочку в той же вкладке с журналом
  и архивом. Это новый процесс без переменных/функций старого и без повторения
  CLI/недоставленных клавиш. Старое ограниченное recovery ADR0018 сохранено.
- `tui_adapters.rs`: выбранные стратегии composer/turn semantics. Смена
  адаптера отменяет текущие proofs, сохраняя архив и уже подтверждённую
  фильтрацию исторических строк. Неизвестные экраны целиком доступны для чтения.
  Для Kimi/Gemini добавлен строго ограниченный box/rules prefill fallback.
- Исправлена обнаруженная GUI гонка Unicode-удаления: между скрытым redraw
  и `ESC[?25h` общее поле теряло ownership. `SourceInput::edit_confirmation`
  и `project_edited_for_adapter` восстанавливают его только по видимому VT,
  точному наблюдаемому удалению и прежним source/geometry/style/adapter.
  Предел 750 мс от первого нажатия; повтор не продлевает его. Меню отменяет
  только edit proof, сохраняя pending slash insertion. Настоящий byte-exact
  fixture — `tui-padded-hidden-delete.vt`; отрицательные проверки сохранены.

**Проверки окончательного кода:**

- `cargo fmt --all -- --check` — passed.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed.
- `cargo test --workspace` — **823 passed**, 0 failed: app 738,
  worker 3, PTY unit 21, ConPTY integration 14, environment integration 2,
  terminal 45. `RUST_TEST_THREADS=4`. Логи —
  `target/profile-pipeline/{fmt,clippy,tests,release}.log`.
- `cargo build --release` — passed. Размер **3 371 008** байт, SHA256:
  `06C08CC5B9B2526241A9EFF05542E2F1EAFFE4BFEE5C9132FA94881B66AFCD58`.
  Метаданные — `target/profile-pipeline/release.json`.
- GUI SingleInput — **211 passed / 29 skipped / 0 failed / 0 error**:
  `target/profile-pipeline/gui-single-final25/run-20261102-214932-0c41bd70/summary.json`.
  Создание и roundtrip профиля, точные argv с пустым значением и пробелами,
  environment/default shell, отсутствие PTY writes от настроек, Unicode,
  выделение/удаление, меню и handoff. Тёмные native controls и раскрытый
  ComboLBox просмотрены на PNG этой сборки.
- GUI TuiTurns — **216 passed / 3 skipped / 0 failed / 0 error**:
  `target/profile-pipeline/gui-turns-final25/run-20261102-214932-636cd1cf/summary.json`.
  Оба GUI manifest подтверждают тот же SHA256, собственные процессы закрыты.
  Итого GUI **427 passed / 32 skipped**; skips — foreground/interactive UIA
  private desktop и человеческая речь NVDA, не пройденная доступность.
- Нативные editor-тесты: 10 passed (в составе app), включая Add/Replace
  аргументов, восстановление черновика, DPI 100/150%, dark popup и HC fallback.
  Private children подтвердили точные argv PowerShell 5/7/Git Bash, допустимые
  CMD args/явный отказ сложных, независимые environment и полный exit drain.
- Один независимый code-review выполнен. Найдено перезаписывание предыдущего
  аргумента при «Добавить»: исправлено и проверено native regression + GUI.
  Дополнительные GUI дефекты bounded prefill/history/delete устранены
  отдельными регрессиями; тестовые ожидания и producer protocol не ослаблены.

**Запуск:** 02.11.2026 21:50:59 +04, PID **21640**, HWND **7867236**,
Responding=true; `target/profile-pipeline/launch.json`. Прежние пользовательские
процессы агент не завершал; список `bks-app*` перед этим запуском был пуст.

**Следующий шаг — ручная приёмка и постепенное развитие отдельных адаптеров.**
NVDA проверяет человек: имена/переходы editor, меню, выделение, речь и звук.
Настоящие ответы Codex/Claude, выход OpenCode и команды новой оболочки также
нужно проверить пользователю. Авторизованные модельные запросы не выполнялись.
Codex 0.164.0/Claude 2.1.240 имеют реальные offline editor captures;
OpenCode 2.0.32 — captures меню/ответов/ошибок. У установленного Kimi 2.4.1
подтверждён только trust-screen без подтверждения; Gemini не установлен,
проверены документация и structural fallback. Не объявлять их полную
индивидуальную интеграцию и NVDA принятыми. Пользовательские auth/config,
CLI installation/update не менялись. Старые сборки не должны редактировать
profiles.json schema 2. Новых известных незавершённых правок кода нет.

### Предыдущая точка остановки — 02.11.2026, после preview.24

Последняя задача реализована: исправлена дополнительная гонка ошибки 232
после OpenCode, доработано общее поле Codex, проверено редактирование Claude.
Собрана и запущена **0.4.0-preview.24**; исполняемый файл —
`target/release/bks-app.exe`, сохранённая копия —
`target/release/bks-app-0.4.0-preview.24.exe`. Все обязательные проверки прошли:
fmt, clippy, 779 workspace-тестов и release; GUI — 418 passed, 32 skipped.
Подробности, ограничения и ссылки на логи приведены ниже.

**Продолжить с ручной приёмки этой сборки:** выход из OpenCode через `/exit`
и первая команда PowerShell сразу после выхода; общее поле Codex/Claude
после настоящего ответа, выделение/удаление и работа с NVDA. Пользователь ещё
не подтвердил результат preview.24; автоматические и offline-проверки такого
подтверждения не заменяют. Незавершённой правки кода в последней задаче нет.
Рабочие изменения сохранены без commit/push; прежние незакоммиченные изменения
нужно сохранить при продолжении.

### Последний результат — 02.11.2026, 0.4.0-preview.24

Продолжение пользовательских снимков 172528/172655. Пользователь уточнил:
ошибка 232 появляется после OpenCode; Codex/Claude — отдельная задача общего ввода.

- Найдена ещё одна гонка handoff: первая ordinary shell-команда снимала
  CliExitWatch сразу после enqueue, до фонового WriteFile. Теперь исходное
  фиксированное окно 5 секунд после возвращения parent prompt сохраняется
  также при следующих enqueue; команды и повторные prompt его не продлевают.
  Exact natural CLR 0x80131623 и полный drain остаются обязательными.
  Ошибка канала сама по себе, normal exit и произвольный профиль не разрешают
  restart. Прежние команды/клавиши не переносятся в новый процесс.
- shell_command_queued разрешает явный пользовательский Clear-Host, пока
  writer исправен. Известная ошибка writer проверяется ДО обработки первого
  финального clear, ещё до обычного state poll. Closing/Failed/Exited также
  сохраняют журнал. Один независимый read-only проход нашёл именно ранний
  clear после enqueue; исправлено, native регрессия проверяет этот порядок.
- Native unprobed writer закрытого ConPTY принимает первую команду в очередь,
  затем даёт настоящий OS pipe error. Добавленный queued-command вариант
  existing recovery regression до fix **failed** (Failed вместо Closing),
  после fix **passed**, с history/draft, одной recovery и отсутствием replay.
  Логи — `target/cli-followup/queue-race-{before,after}.log`.
  Новый отдельный тест запрещает продление исходного handoff deadline.
- Codex padded composer не распознавался при footer сразу после единственной
  padding-строки или при правом footer (weekly limit на пользовательском
  снимке). plain_padded_shape теперь допускает обе геометрии, сохраняя строгие
  margins/единственность/границы. Literal hint больше не попадает в EDIT.
- Удаление префикса подтверждённого draft до caret0 больше не превращает
  оставшийся строгий суффикс с тем же source ink в новый hint. Отрицательные
  проверки новых hints, source identity и palette сохранены. SourceInput и
  main не требовали правок: существующий ACK/selection путь подходит.
- Получены настоящие offline editors Codex 0.164.0 и Claude 2.1.240, 100×29:
  Unicode, Home, Right, Backspace, End, clear, slash, Down/Esc. Все 10 endpoints
  обоих CLI alive/без exit status, собственные процессы закрыты. Ни Enter,
  ни LLM-запрос, ни пользовательский auth/config не использованы. Claude
  --bare с private onboarding/trust и fixed fake API key на закрытом loopback.
  Подтверждённого отдельного дефекта Claude в этих сценариях не обнаружено.
  Три новые byte-exact fixtures и metadata —
  `crates/bks-app/tests/fixtures/tui-codex-claude-composers/`;
  4 новые projection-регрессии, projection54/source_input10 passed.
  Рабочие captures — `target/codex-input/` (не считать startup с длинным
  daemon socket доказательством live editing; реальные editor traces имеют
  короткий private CODEX_HOME). Native GUI дополнен generic padded/ruled
  shapes с right footer, source-held ACK, Unicode, Home/Right, selected
  Backspace и очисткой, с тем же HWND и UIA text/native selection.

**Проверки:** fmt, clippy `-D warnings`, workspace **779 passed**
(app697 + worker3 + PTY19unit/15integration + terminal45), release passed.
Итоговый cargo test --workspace выполнен с RUST_TEST_THREADS=4. Один предыдущий
session-only parallel run дал 58 passed/1 failed: EXIT_SEVEN остался Running
к deadline15s; итоговый полный прогон прошёл без изменения этого теста.
Логи — `target/cli-followup/{fmt,clippy,tests,release}.log`.
Просмотрены затронутые изменения и git diff --check, passed.

Настоящий OpenCode через npm opencode.ps1 БЕЗ --standalone с собственным
service port46397: /exit → natural CLR → recovery → unique echo, passed,
`target/cli-followup/opencode-fast-actual.log`. Собственный service остановлен.
Настоящий npm Codex с private коротким CODEX_HOME: /quit → прежняя PowerShell
Running → unique echo, passed, `codex-exit-actual.log`. Ранняя проба с длинным
daemon socket и первый stale Cargo artifact не являются exit acceptance.
Два собственных Codex daemon с exact private socket path остановлены.
Два daemon/worker editor-capture также остановлены; повторный CIM-аудит
дал 0 собственных оставшихся процессов, `target/codex-input/cleanup-result.json`.

Release GUI (SHA совпадает с EXE), paths от `target/cli-followup/`:

- `gui-single/run-20261102-180806-0a68186e/summary.json`
  — **202 passed / 29 skipped / 0 failed/error**.
- `gui-turns/run-20261102-180806-7e7c88ed/summary.json`
  — **216 passed / 3 skipped / 0 failed/error**.

Skips — interactive UIA selection/caret, physical input/foreground/NVDA;
не passed. Авторизованные ответы Codex/Claude и NVDA принимает человек.
Восстановление PowerShell сохраняет журнал/черновик, но теряет локальные
переменные, cwd берётся из профиля; это новый процесс.

**Сборка:** `target/release/bks-app.exe` и `bks-app-0.4.0-preview.24.exe`,
3 132 928 байт, SHA256
`FA4D85E31BDF1734C4C5D6444ED1B960FA8608ECC253CC2596AE2A4759796972`.
EXE открыт 18:11:28 +04:00, PID21244, HWND3738206, responding/input-idle true.
Метаданные — `target/cli-followup/{release,launch}.json`.
Версия, changelog, manual-final, known-limitations и ADR0018 обновлены.
Commit/push и упаковка не выполнялись.

### Предыдущий результат — 02.11.2026, 0.4.0-preview.23

Исправлена гонка выхода OpenCode → PowerShell по снимку 154411.
Сообщение «Идет закрытие канала» — Win32 232; также обработаны 109/233.

- Ранний writer-close вызывал `fail()` → TerminateProcess(1) до natural
  0x80131623, теряя eligibility. UI handoff и stopped-input cleanup также
  стирали auto_cli-флаги. Теперь отдельный CliExitWatch принадлежит текущему
  PTY и принятому CLI/parent prompt стандартной локальной pwsh. Возврат
  prompt ограничивает watch пятью секундами, обычная принятая команда снимает
  его. UI-флаги больше не являются единственным доказательством владения.
- При подходящем native pipe-close ввод отключается, но до 5 секунд от
  первой ошибки продолжаются nonblocking poll/drain вместо принудительного
  убийства. Повторные ошибки не продлевают дедлайн; timeout работает и без
  новой ошибки. Только exact CLR-code и полный drain допускают одну новую
  оболочку. Normal exit, CLI-only/custom startup/remote и отсутствие ownership
  не перезапускаются. Клавиша/команда в момент закрытия не повторяются.
- `win_err` сохраняет Win32 HRESULT как raw OS code через writer queue,
  вместо Other/string без кода. Другие HRESULT сохраняют прежний формат.
  Проверены настоящий закрытый ConPTY writer и WindowsError 109/232/233.
- User-write замечает закрытие до обычного poll, не создаёт ложный modal
  для ожидающего завершения; сообщения различают Closing, Starting и готовую
  оболочку. Журнал не очищается финальным CLI clear в Closing/Failed/Exited
  и свежем handoff. Новая явно принятая Clear-Host сохраняет обычное действие.
- Добавлены 6 тестов: один в PTY и пять session. Native fault injection
  проверяет pipe-close до/после prompt handoff, VT-query, late user key,
  точный natural exit, отсутствие replay, history/draft и успешную новую
  команду. Также fixed timeout, истёкший watch, новая обычная команда и
  teardown clear после exit status без PendingChannelClose.
- Один независимый read-only code-review выявил потерю журнала при late clear.
  Отрицательная проверка — `target/cli-exit/review-journal-before.log` (failed);
  замечание исправлено, затронутые проверки прошли. Других подтверждённых
  замечаний в этом проходе не было; полный повторный круг не выполнялся.

**Проверки:** fmt, clippy `-D warnings`, workspace tests **774 passed**
(app692 + worker3 + PTY19unit/15integration + terminal45), release passed.
Логи — `target/cli-exit/{fmt,clippy,tests,release}-final.log`.

Настоящий установленный OpenCode 2.0.32 в отдельной стандартной PowerShell:
`/exit` → natural 0x80131623 → восстановление → уникальный составной echo,
**passed**, `target/cli-exit/actual-final.log`. Helper — `probe/`, частные
данные — `isolated/` внутри того же каталога. Без LLM, внешних запросов,
пользовательского профиля; собственные процессы закрыты. Чистый baseline
иногда уже работал; перестановку раннего pipe-close воспроизводят native
регрессии, а не утверждение о постоянном падении чистого запуска.

Release GUI, SHA совпадает с открытым EXE, пути от `target/cli-exit/`:

- `gui-single-input/run-20261102-170405-449ade9c/summary.json`
  — **160 passed / 21 skipped / 0 failed/error**.
- `gui-turns/run-20261102-170406-d73a6edd/summary.json`
  — **216 passed / 3 skipped / 0 failed/error**.

Skips — foreground/physical input/interactive UIA selection/NVDA; не passed.
NVDA принимает человек. Восстановление создаёт новый процесс: переменные и
произвольный cwd прежней оболочки не восстанавливаются, используется профиль.

**Сборка:** `target/release/bks-app.exe` и `bks-app-0.4.0-preview.23.exe`,
3 131 392 байт, SHA256
`1F94BB7BB6BBF091766A4A4D47925A0F59E4EAC5B10B06C16B6536F9545DCB24`.
Метаданные — `target/cli-exit/release.json`. EXE открыт в 17:07:34 +04:00,
PID 21520, HWND 1182454, responding/input-idle true; `target/cli-exit/launch.json`.
Обновлены версия, changelog, ADR0018 и ручная приёмка. Пакеты, commit/push
не делались; пользовательские процессы не завершались.

### Предыдущий результат — 02.11.2026, 0.4.0-preview.22

Исправлены три замечания по снимкам 141957/142017/142024: повторный чёрный
фон меню, потеря финальной ошибки в чтении, удаление локального выделения TUI.

- В `TuiSurface` добавлена проверка периметра live alternate screen. Большой
  popup и несколько панелей делили фон на оттенки без единственного большинства,
  из-за чего тема отключалась. Нужны >1/2 края одного нейтрального truecolor,
  >=1/8 площади этого оттенка и >1/2 общей нейтральной площади. Тонкая рамка
  вокруг ANSI не принимается за канву. Прежний dominant-shade fallback сохранён.
  Grid и native-reader используют одну политику. Проверены Model/Variant,
  error-подложка и source-equivalent панели при 100x29.
- `read_panel_body` больше не вырезает красную финальную ошибку перед elapsed
  summary: полный текст/стили сохраняются, retry заменяется, active закрывается
  со статусом «Ошибка TUI», successful Ready не создаётся. `AnswerTarget.failed`
  используется также в объявлении перехода к ответу. Новый одинаковый prompt
  получает ownership при сохранённом старом live-body и неизменном footer
  после нового echo на тех же строках. Защиты ED2/history/quote/code сохранены.
- `NativeSelectionEdit` переводит локальное выделение EDIT в движение исходной
  каретки и Backspace, только после точного подтверждения каждого шага через VT.
  Source scalar/grapheme/CRLF не считается числом UTF-16 клавиш. Набор/вставка
  после удаления сохраняет порядок. Source/session/focus/menu/protected,
  timeout 750 мс без ack и ошибка записи отменяют очередь; она не переносится
  в другой источник. Обычные клавиши unknown/protected/без выделения сохраняют
  прежнюю обработку. GUI fixture теперь поддерживает реальную каретку и deletion.
- Добавлены 15 тестов: 5 по материалу/подменю/reader, 6 по selection edit,
  4 по error/archive ownership. Реальные captures OpenCode 2.0.32:
  `tui-opencode-menu-stack.vt`, `tui-opencode-error-menu.vt` и
  `tui-turns-opencode-errors.vt`; endpoints/SHA — в соседних `.md`.
  Ошибки от фиксированного loopback HTTP 503→403, два «привет», никакого
  внешнего LLM, пользовательских профилей или инструментов. Собственные
  процессы/слушатели закрыты. Диагностический запуск без standalone показал
  только Starting background server и не использовался для приёмки UI.
- Исходное отрицательное воспроизведение материала —
  `target/tui-followup/menu-before.log`; просмотрены native GDI preview
  `menu/{panelled-models,variant,error-model-menu}.png`. Это непрозрачный
  рендер записей/производной подложки, не ручная DWM/NVDA-приёмка.

**Проверки:** fmt, clippy `-D warnings`, workspace tests **768 passed**
(app687 + worker3 + PTY18unit/15integration + terminal45), release passed.
Логи — `target/tui-followup/{fmt,clippy,tests,release}-final.log`.
Release GUI, SHA совпадает с открытым EXE:

- SingleInput: `gui-single-input/run-20261102-145002-0cd2cb69/summary.json`
  — **160 passed / 21 skipped / 0 failed/error**.
- TuiTurns: `gui-turns/run-20261102-145002-a9fa9b44/summary.json`
  — **216 passed / 3 skipped / 0 failed/error**.

Оба пути относительно `target/tui-followup/`. Skips — foreground/physical
keyboard/интерактивное UIA selection и человеческая NVDA-приёмка; не passed.
SingleInput проверяет selected Unicode+CRLF Backspace/Delete, fast replacement,
held ack, focus cancel и отсутствие синтетики на unknown/protected source.
TuiTurns проверяет текущий UI чтения/архива и возвраты. Legacy PaintIsolation
не запускался повторно. Два узких агента помогли с вводом и error ownership;
интеграция, просмотр изменений и обязательные проверки выполнены основным.

**Сборка:** `target/release/bks-app.exe` и `bks-app-0.4.0-preview.22.exe`,
3 127 296 байт, SHA256
`E20A607E2C8651BE5F77A879603CDBBD18E203846A6E515869B814C771BC975F`.
Метаданные — `target/tui-followup/release.json`. EXE открыт в 14:52:47 +04:00,
PID 21588, HWND 9046332, responding/input-idle true; `target/tui-followup/launch.json`.
Обновлены версия, changelog, ADR0014/0017 и ручная приёмка. Пакеты, commit/push
не делались; пользовательские процессы не завершались.

### Предыдущий результат — 02.11.2026, 0.4.0-preview.21

Исправлено оформление OpenCode по снимку `Снимок экрана 2026-11-02 123958.png`.
Пользователь уточнил объём: чёрные фоны и резкие цвета меню, именно интерфейс.

- `TuiSurface` определяет материал по полному текущему VT-кадру, включая
  пустые клетки. Этот результат используют GDI-сетка и native RichEdit;
  прежде GDI вообще не применял адаптацию нейтрального truecolor-фона.
  Доминирующий фон становится графитовым материалом терминала, вторичные
  нейтральные панели сохраняют иерархию. Grid-политика ограничена полным live
  alternate screen; shell и scrollback за такой экран не принимаются.
- Яркая полоса в однозначной плавающей панели получает 22% синего акцента
  относительно её фона; тёмный текст выбранного пункта становится читаемым.
  Требуются боковые поля, нейтральное окружение, непрерывные строки панели
  сверху/снизу и уникальная заливка. Неоднозначные, обрезанные, inverse и
  повторные полосы сохраняют исходный акцент. Это оформление без распознавания
  действий. Геометрия, VT-атрибуты, обычные ANSI-цвета, ввод и selection не меняются;
  HC сохраняет системную обработку. Кеш native-reader учитывает смену политики
  даже при одинаковых spans; закрытие/resize не оставляет прежнюю палитру.
- Записан установленный OpenCode 2.0.32 в отдельном ConPTY: три локальные
  тестовые модели, открытие `/models`, Down, Esc, resize 110x36→80x24 и повтор.
  Запросы модели, инструменты и пользовательские профили не использовались;
  внешний доступ отключён, собственный процесс завершён. Запись 39 456 байт —
  `crates/bks-app/tests/fixtures/tui-opencode-model-menu.vt`, endpoints и SHA
  описаны в соседнем `.md`. Временный helper — `target/opencode-ui/capture/`.
- Добавлены пять регрессий: три для палитры/неоднозначных форм/границ кадра,
  native RichEdit для цветов, сохранения текста/selection и смены политики,
  native GDI для пяти состояний меню, движения, закрытия и resize. Последний
  повторно использует нарисованный target и проверяет каждый пиксель пустых
  клеток: новые поверхности сами по себе не доказывают очистку старых букв.
  Отрицательное воспроизведение — `target/opencode-ui/native-before.log` и
  `models-before.png`. Просмотрены `models.png`, `models-down.png`, `closed.png`
  и `narrow-models.png`: это настоящая непрозрачная GDI-отрисовка записи,
  без утверждения о ручной приёмке DWM/NVDA.

**Проверки:** fmt, clippy `-D warnings`, workspace tests **753 passed**
(app672 + worker3 + PTY18unit/15integration + terminal45), release passed.
Логи — `target/opencode-ui/{fmt,clippy,tests,release}-final.log`.
Актуальный `-SingleInput` release GUI:
`target/opencode-ui/gui-single-input/run-20261102-140330-30af4d36/summary.json`
— **128 passed / 16 skipped / 0 failed/error**. SHA GUI совпадает с открытым EXE.
Skips относятся к foreground/физическому вводу и человеческой NVDA-приёмке.

Дополнительный `-PaintIsolation` был выбран ошибочно: header скрипта относит
его canvas/companion-контракт к preview.8/9. Он завершился **error** на ожидании
скрытых legacy HWND и на preview.21, и на контрольном preview.20; это не passed.
Логи — `gui-paint*`; временная перестановка запроса paint-scene не помогла и
отменена. Скрипт не изменён этой итерацией; приёмка новой версии — SingleInput
и перечисленные native-регрессии. Не продолжать старый сценарий без отдельной
адаптации к текущему native-reader, который виден вместо legacy overlay.

**Сборка:** `target/release/bks-app.exe` и `bks-app-0.4.0-preview.21.exe`,
3 104 768 байт, SHA256
`CE98AF5FDC3B1F2A9A12C6AE1D878E7AC62443ECB5F92746093C7D2D7E487959`.
Метаданные — `target/opencode-ui/release.json`. EXE открыт в 14:14:17 +04:00,
PID 908, HWND 6161406, responding/input-idle true; `target/opencode-ui/launch.json`.
Обновлены версия, changelog, ADR0017 и ручная приёмка. Пакеты, commit/push
не делались; пользовательские процессы не завершались.

### Предыдущий результат — 02.11.2026, 0.4.0-preview.20

Добавлена мягкая подсветка строки с кареткой и выделения текста в стиле
Frosted Glass. Пользователь подтвердил предыдущие исправления preview.19.

- В активном документе чтения строка с пустым выделением получает 8% синего
  акцента; при уходе фокуса полоса исчезает. Геометрия берётся из нативного
  TOM, поэтому подсветка следует за реальными строками, кареткой и прокруткой.
- Выделение в документе и поле команды — 22% акцента, неактивное — 14%.
  Обычные Shift/Ctrl+Shift+стрелки и остальные native-сочетания сохраняются.
  `text_decorations.rs` меняет только пиксели: текст, диапазоны, форматирование,
  фокус и доступный HWND остаются нативными. Нет новых клавиатурных обработчиков.
- Альфа восстанавливается перед наложением оформления; сглаженные края
  выделенных букв пересчитываются поверх мягкого фона без ярких синих ореолов.
  Ядра букв и исходные ANSI-атрибуты сохраняются. Подсветка строки не заливает
  явные цветные ANSI-фоны. Непрозрачный режим использует ту же палитру;
  High Contrast сохраняет системную отрисовку и выделение.
- Добавлены восемь тестов: четыре для геометрии/пикселей, три с реальными
  RichEdit/EDIT и один для альфы GlassBuffer. Native fixtures сериализованы
  общим существующим mutex на время жизни HWND. Просмотрены образцы
  `target/text-highlights/caret-row.png`, `text-selection-final.png` и
  `input-selection-opaque.png`: нативный рендер на нейтральном фоне,
  без утверждения о ручной DWM/NVDA-приёмке.
- Уточнён контракт live PowerShell-регрессии из preview.19: при неоднозначном
  повторном эхо PSReadLine сохраняется доступная исходная граница блока.
  Тест проверяет три разных непустых начала, принадлежность полного результата
  своей команде и все переходы. Строгое начало на первой строке результата
  остаётся в captured-echo и GUI caret/TextPattern-проверках. Только тестовая
  оболочка ждёт устойчивого prompt, импортирует PSReadLine и отключает запись
  его истории. Production-отправка команд не менялась. Ранние провалы старого
  ожидания сохранены в `tests-initial-failed.log` и `tests-before-echo-contract.log`.

**Проверки:** fmt, clippy `-D warnings`, workspace tests **748 passed**
(app667 + worker3 + PTY18unit/15integration + terminal45), release passed.
Логи — `target/text-highlights/{fmt,clippy,tests,release}-final.log`.
Release GUI: `target/text-highlights/gui-release/run-20261102-132929-073d2db9/summary.json`
— **128 passed / 16 skipped / 0 failed/error**. Сохранились проверки Esc,
трёх блоков, исходного TUI-ввода и UIA. Skips — foreground, физический ввод
и человеческая NVDA-приёмка на private desktop; они не объявлены пройденными.

**Сборка:** `target/release/bks-app.exe` и `bks-app-0.4.0-preview.20.exe`,
3 098 112 байт, SHA256
`AF4F753631327E0F007FF81ADDE2A8048E6ECAA0F36387DEB4B464460FF1EDF5`.
Метаданные — `target/text-highlights/release.json`. EXE открыт в
13:31:10 +04:00, PID 18408, HWND 9045636, responding/input-idle true;
`target/text-highlights/launch.json`. Обновлены версия, changelog и ручная
приёмка. Пакеты, commit/push не делались; пользовательские процессы не завершались.

### Предыдущий результат — 02.11.2026, 0.4.0-preview.19

Исправлены два замечания по навигации shell: каретка выше результата и
пропадающий третий блок в последовательности `ls`, `docker ps`, `ls`.

- CommandBlocks больше не фиксирует окончательное начало на пустой строке
  сразу после эха: дожидается непустой строки и сохраняет её отступы.
  Переносы/пустые строки между частями эха учитываются. Для коротких
  однострочных integrated-команд поддержаны части эха, которые ConPTY
  повторно не передал; generic/no-echo shell сохраняет исходный текст.
  Завершённый блок с неизвестным эхо остаётся доступен от своей границы;
  поиск ограничен его концом и не принимает следующий prompt за результат.
- В обычном shell TerminalOutput добавляет границу абзаца, когда ConPTY
  сериализует вертикальный переход управляющим кодом вместо CRLF. Это
  устраняет склейку команды с первым результатом. Direct/auto CLI/alternate
  screen отключают преобразование; это не полная VT-проекция журнала.
- Добавлены пять регрессий CommandBlocks, две для переводов строк/стилей/
  UTF-16 offsets и реальный ConPTY/PowerShell-тест трёх команд. В последнем
  docker — локальная функция только тестовой оболочки с постоянной ошибкой;
  пользовательский Docker/сеть не вызываются. Восемь повторных прогонов
  после добавления границ строк прошли (`session-with-lines-1..8.log`).
- GUI-проверка создаёт три длинных результата, подтверждает реальные native
  selection и UIA Line на первой строке каждого, переходы 3→2→1→2→3,
  края, вход из command field в последний/первый, сохранение черновика и
  нулевые PTY-байты навигации. Используются private F7/F8 aliases тех же
  PreviousBlock/NextBlock, что Ctrl+Up/Down, без глобального ввода и изменения
  пользовательских настроек. Физический Ctrl и речь NVDA остаются ручными.

**Проверки:** fmt, clippy `-D warnings`, workspace tests **740 passed**
(app659 + worker3 + PTY18unit/15integration + terminal45), release passed.
Логи — `target/output-block-fix/{fmt,clippy,tests,release}-final.log`.
Отрицательное исходное воспроизведение — `session-baseline.log`; промежуточные
GUI-прогоны в `gui-debug*`/`gui-navigation-diagnostics` не считать приёмкой.
Финальный debug GUI — **128 passed / 16 skipped / 0 failed/error**.
Финальный release GUI: `target/output-block-fix/gui-release/run-20261102-123132-3ae91624/summary.json`
— **128 passed / 16 skipped / 0 failed/error**. Skips относятся к foreground,
физической каретке/вводу и человеческой NVDA-приёмке на private desktop.

**Сборка:** `target/release/bks-app.exe` и `bks-app-0.4.0-preview.19.exe`,
3 092 480 байт, SHA256
`38A3DA1580FCDF44FBD682E8D4B7A3DCA00D3B9D4C24CC11000CEBB2FFF5ED83`.
Метаданные — `target/output-block-fix/release.json`. Проверенный EXE открыт
в 12:33:12 +04:00, PID 16848, HWND 4195416;
`target/output-block-fix/launch.json`. Пользовательские процессы не завершались.
Обновлены версия, changelog и ручная приёмка. Пакеты, commit/push не делались.

### Предыдущий результат — 02.11.2026, 0.4.0-preview.18

Исправлено закрытие главного окна по Esc в обычном поле команды и журнале.
На preview.17 нажатие в поле завершало процесс с кодом 0. Нативная
диалоговая обработка Windows/RichEdit отправляла WM_CLOSE. Одного обхода
IsDialogMessage оказалось недостаточно: повторный Esc отправлял WM_CLOSE
уже через нативную обработку поля. Теперь dispatch_main_message поглощает
необработанные Esc key/char до обоих путей. Обработчики поиска, вспомогательных
окон, чтения TUI и исходного TUI-ввода выполняются раньше и сохраняют отмену.

Добавлена нативная регрессия с настоящими editable/read-only RichEdit:
три Esc с корректными key-up не создают WM_CLOSE/IDCANCEL и не меняют текст.
Test-ShellEscape в check-unified-tui.ps1 проверяет очередь сообщений (прямой
SendMessage ошибку не воспроизводил), повторный Esc, поле/журнал/главное окно/
кнопку новой вкладки, сохранение черновика/выделения/журнала, локальную отмену
поиска/справки и отсутствие PTY-байтов до TUI и после возврата из CLI.
Прежняя проверка check-stage5 покрывала Esc только в прямом вводе.
Обновлены текущая версия, changelog и ручная приёмка.

**Проверки:** fmt, clippy `-D warnings`, workspace tests **732 passed**
(app651 + worker3 + PTY18unit/15integration + terminal45), release passed.
Логи: `target/escape-fix/{fmt,clippy,tests,release}-final.log`.
Отрицательные нативные воспроизведения сохранены в `native-before.log` и
`native-repeated-before.log`; первый GUI-кандидат остановлен на повторном Esc
(13 passed / 1 failed) в `single-input/`, финальный прогон — отдельный.

**GUI:** `target/escape-fix/single-input-final/run-20261102-112920-079b926f/summary.json`
— **115 passed / 16 skipped / 0 failed/error**. Подтверждены отмена поиска и
справки, сохранение shell, ровно один `1B` в TUI-меню и возврат к shell.
Skips — foreground-gated focus и человеческая NVDA/физическая приёмка на private
desktop; они не объявлены пройденными. Дополнительный скрытый процесс с настоящим
PowerShell выдержал девять Esc, включая тройные в поле и журнале:
`target/escape-fix/native-smoke-final.log`. Пользовательские сессии не завершались.

**Сборка:** `target/release/bks-app.exe` и `bks-app-0.4.0-preview.18.exe`,
3 088 384 байт, SHA256
`D61D821FD5E9E3E937DFA3C83400248C06693B97D5CC7C8BE9FBA4BB0B4126F1`.
Метаданные — `target/escape-fix/release.json`. Проверенный EXE открыт для
пользователя в 11:31:04 +04:00, PID 15280, HWND 5572952;
`target/escape-fix/launch.json`. Пакеты, commit/push не делались.

### Предыдущий результат — 31.10.2026, 0.4.0-preview.17

Продолжена унификация Kimi/OpenCode/Codex/Claude по замечаниям о чтении
OpenCode и отдельном поле Codex:

- Физические строки TUI сохраняются в одном источнике для чтения, поиска
  и VisualDocument. Смена фокуса не склеивает страницу и не возвращает
  исходную чёрную заливку. Тема действует также в архиве ответов. Bookmark
  сохраняется на native EN_KILLFOCUS; снят запрет follow для visual reader.
- OpenCode Markdown распознаётся после точного принятого запроса в панели.
  Архив хранит отдельные ответы без chrome, сохраняет длинное продолжение
  после ухода prompt за экран. Наблюдаются все завершённые synchronized
  frames внутри PTY drain, без искусственного продвижения времени и
  промежуточной публикации UI/речи. Retained composer без caret не доказывает
  готовый безметочный кадр: это устранило порчу на captured endpoint 52801.
- Общий composer поддерживает Codex с отступами, `›`, цветным и бесцветным
  фоном; Claude с горизонтальными границами и NBSP; Unicode, переносы,
  пользовательские отступы и source-owned caret. `SourceInput` допускает
  CR/LF только внутри полностью принятой bracketed paste, прежние лимиты
  подтверждения сохранены. Меню/команды не исполняются по распознанному тексту.
- В native QA найден и исправлен отдельный ConPTY hidden-cursor переход:
  SourceInput хранит исходный Composer только внутри bounded pending input.
  `project_recovered` требует точного accepted insertion/prefix и caret,
  не допускает palette-only подтверждения. Public hidden composer остаётся
  None, поэтому private proof не попадает в анализ ответа. Partial ack ждёт
  полного text/caret; clear/expiry/control/source change удаляют proof.
- Для стрелок/отмены меню есть отдельная private continuity 750 мс:
  только точное неизменное поле с тем же стилем и формой, допустимы его
  вертикальный перенос и текущая source caret. Скрытый blank кадр остаётся
  публично неопределённым. Visible frame/expiry/resize/mode/cancel снимают proof.
  Реальный isolated ConPTY capture проверяет slash → Down и выбранный пункт,
  а отрицательные случаи не разрешают восстановление по одному цвету.
  Exact continuation имеет приоритет над ordinary placeholder: при Home
  прежний Unicode больше не становится пустым hint. Отдельный ConPTY capture
  проверяет текст и UTF-16 caret 4→0→1; hidden source-roundtrip инвалидирует proof.
- Парные форматы Codex/Claude получили точное разделение запросов/ответов,
  возврат ввода и typed/DIM completion. Обычная строка `Error:` не обрывает
  ответ: нужен отдельный оформленный final error block. OSC10/11/12 получают
  актуальные default цвета темы/HC; clipboard и другие OSC не добавляют действий.
- Решение описано в ADR0019; обновлены help/manual/limitations/changelog.

**Проверки:** fmt, clippy `-D warnings`, workspace tests **731 passed**
(app650 + worker3 + PTY18unit/15integration + terminal45), release passed.
Логи `target/frosted-indigo/{fmt,clippy,tests,release}-preview17-final.log`.
Первый workspace run выявил конфликт параллельных короткоживущих RichEdit
fixtures. Общий test-only mutex изолирует их полный native lifetime,
соответствуя одному UI thread приложения; повторный полный прогон прошёл.
Recovery fixture ждёт уникальную фактическую строку вывода, затем задаёт
completed CommandBlocks по её точным UTF-16 границам и проверяет одинаковые
start/end/text после native recovery. Определение функции без вывода и
ConPTY echo с лишним backslash не гарантировали автоматический output anchor;
этот отдельный echo contract не подменяется проверкой сохранения.
Независимое узкое review: исправлены потеря bookmark, follow и ложный Error;
каждое исправление перепроверено. `git diff --check` passed.

**Приёмка** (пути внутри `target/frosted-indigo/`):
- `preview17-final4-single-input/run-20261031-222245-6461ccb4/summary.json`:
  **99 passed / 16 skipped / 0 failed/error**.
- `preview17-final4-tui-turns/run-20261031-222245-b819b39c/summary.json`:
  **216 passed / 3 skipped / 0 failed/error**. Skips — foreground-gated focus,
  физическая каретка/звук/NVDA на private desktop.
- `codex-claude-composer/qa-result-preview17.json`: на финальном SHA цветной
  Codex, бесцветный Codex и Claude завершили по **25 variant checks passed /
  7 skipped / 0 failed/error**. Проверены одно нижнее поле без верхнего дубля,
  Unicode, Home/Right/End, однократный Send, source menu/Down/Escape; шесть PNG
  просмотрены. Для plain/Claude отдельные полные прогоны — **36 passed** каждый.
  Первоначальный общий run **57 passed / 1 error** сохранён: меню запрашивалось
  после промежуточного пустого поля до подтверждения очистки источником.
  Отдельные прогоны ждут завершения собственного render и устойчивого empty
  source не менее 150 мс. Быстрый ввод сразу после Send не считается проверенным.
- `opencode-reader-qa/qa-result-recovered.json`: **93 checks passed** на втором кандидате — два ответа,
  latest/prev/next без footer, 31 отдельная UIA-строка, одинаковые HWND/цвета,
  bookmark после native targeted click и сохранение старого ответа при новом.
  Два PrintWindow PNG просмотрены. Дополнительный EOF-follow сценарий
  **не принят**: исходная assertion неверно считала padded EOF; на втором
  кандидате corrected сценарий остановился на предусловии native EOF/scrollbar
  bottom до нового кадра. В сыром summary **93 passed / 1 failed / 0 error**;
  runtime follow не был проверен и не скрыт как pass.
  GUI проверял полные styled snapshots из реального capture: повторная подача
  уже сериализованного ConPTY delta через второй consolehost неэквивалентна.
  Byte-exact поток отдельно прошёл core/session regressions, включая 120
  маркеров по одному и одинаковый результат при 4096/65536-байтовых reads.

Нативная вставка многострочного текста без clipboard не считается принятой
по synthetic IPC: fixture сам менял текст без accepted PTY input, поэтому
unknown changed field после hidden кадра правильно не использовал старое proof.
SourceInput/projector tests отдельно проверяют полный bracketed paste,
Unicode/CRLF/отступы и точное source подтверждение. Clipboard пользователя
не читался и не изменялся.

Codex/Claude authenticated responses и NVDA не проверены: изолированный Codex
остановился на входе, Claude — на недоступной сети; source-equivalent fixtures
основаны на конкретных версиях renderer. Private desktop не подтверждает
физический ввод мышью/клавиатурой, реальные DPI или DWM-композитирование.
Пользовательские сессии и настройки не изменялись.

**Сборка:** `target/release/bks-app.exe` и `bks-app-0.4.0-preview.17.exe`,
3 088 384 байт, SHA256
`11CE545B4374308B0A77DA86E6F138D01B896BC8C57F95C874E4DF196556E7A0`.
Метаданные `target/frosted-indigo/release-preview17.json`.
Финальный EXE запущен для пользователя в 22:37:38 +04:00, PID 19848,
HWND 5703848; запись `target/frosted-indigo/launch-preview17.json`.
Пакеты, commit/push не делались; прежние незакоммиченные изменения сохранены.

### Предыдущий результат — 31.10.2026, 0.4.0-preview.16

Закрыты замечания по скриншотам 19:12–19:16:

- Убрана короткая голубая линия под вводом. В обычной теме поле продолжает
  стеклянную поверхность, фокус показывает native caret; HC сохраняет рамку.
- OpenCode задавал собственную truecolor-заливку, поэтому его область оставалась
  чёрным прямоугольником. `text_style::TuiSurface` адаптирует доминирующий тёмный
  нейтральный live canvas к фону терминала после разрешения исходных цветов.
  Вторичные панели остаются различимыми матовыми; foreground/DIM, named/indexed
  ANSI, цветные заливки, inverse, равные fg/bg и HC сохраняются. История/VT не
  изменяются; режим чтения возвращает исходные цвета. Обновлён ADR0017.
- На настоящем изолированном OpenCode 2.0.32 воспроизведён CLR FailFast родительской
  PowerShell 7 (`0x80131623`, отключённый console handle), а не просто застрявший
  prompt. При остановке снимаются direct input/capture. Для точного сбоя после
  явно принятого interactive CLI предусмотрена одна попытка нового PTY в той же
  вкладке: только локальный pwsh со строго штатными PS_PROMPT args, после полного
  drain. Журнал, черновик и архив сохраняются; pending source/input не переносятся,
  команды не повторяются. Startup clear не стирает историю. Старый PTY удаляется
  существующим bounded worker pool. Ordinary exit/custom startup/CLI-only и другие
  коды не перезапускаются. Main синхронизирует поле/имя/кнопку и заменяет ошибку
  ввода в остановленный процесс статусом. Решение: ADR0018.

**Проверки:** fmt, clippy `-D warnings`, workspace tests **693 passed**
(app616 + worker3 + PTY18unit/15integration + terminal41), release build passed.
Логи: `target/frosted-indigo/{fmt,clippy,tests,release}-preview16-final.log`.
Самопроверка и один узкий независимый code-review без подтверждённых дефектов;
`git diff --check` passed. Расположение элементов не менялось.

**Приёмка** (пути внутри `target/frosted-indigo/`):
- `opencode-glass-style/final-preview16/summary.json` — **24 passed**, 0 failed/skip.
  Два actual screen A/B: прежний black canvas, незаполненный вывод и ввод меняются
  с фоном; glyph cores/viewport/text стабильны, bright underline pixels=0.
  Оба PNG просмотрены. Secondary matte panels/selection/exact reader colors
  дополнительно проверены native DIB regression; A/B — replay VT, не live CLI.
- `lifecycle/recovery-actual/summary.json` — настоящий isolated OpenCode без
  модельных запросов: startup → /exit → реальный pwsh crash → same-tab recovery →
  успешная новая echo-команда. History/draft сохранены, старые команды не повторены.
- `accent-probe/lifecycle-ui/B7129A50DAA9/summary.json` — **18 passed / 1 skipped**,
  0 failed. Реальный final EXE на private desktop: own FailFast → тот же HWND/tab,
  EDIT readonly/MSAA name/label/button восстановлены, новая команда работает;
  ordinary exit не перезапускается, символ/Execute не открывают modal.
  Physical Enter skipped: private desktop не доказывает физический ввод.
- `preview16-single-input-final/run-20261031-194656-b27184ac/summary.json` —
  **99 passed / 16 skipped / 0 failed/error**, skips foreground/UIA caret private desktop.

Восстановленная оболочка — новый процесс: локальные переменные утрачены, cwd
берётся из профиля (достоверного текущего cwd нет). NVDA, реальные DPI-мониторы,
физический Enter и сетевой ответ модели в этой итерации не проверялись.
Все собственные QA-процессы/окна завершены; пользовательские сессии не закрывались.

**Сборка:** `target/release/bks-app.exe` и `bks-app-0.4.0-preview.16.exe`,
3 016 192 байт, SHA256
`B7129A50DAA9B18BC0755B0DAC4B28A4EA22D5F059947BF03C521EB6A8C4A267`.
Метаданные `target/frosted-indigo/release-preview16.json`.
Пакеты, commit/push не делались; прежние незакоммиченные изменения сохранены.

### Предыдущий результат — 31.10.2026, 0.4.0-preview.15

Закрыты замечания по трём скриншотам 18:07–18:09 и мерцанию Kimi:

- Убрана обводка поля ввода, оставлен короткий индикатор фокуса. В High Contrast
  сохраняется рамка. `styled_scrollbar` рисует тонкий thumb и стеклянную дорожку
  поверх штатной NC-прокрутки, не меняя range/hit targets/команды/native HWND.
- `ui_menu` оформляет EDIT/RichEdit контекстные меню через общий native HMENU
  renderer. `ui_message` заменяет пять app MessageBox sites на themed native modal
  dialogs с векторными пиктограммами; общие dialogs/Help получили те же цвета.
  Default/cancel, MSAA/native captions сохранены. Отдельно исправлен owner-draw
  button protocol: Tab→«Нет»→Enter не должен выполнять default «Да».
- OpenCode больше не схлопывается из-за WRAPLINE + native word-wrap пробелов:
  `tui_projection::VisualDocument` сохраняет физические строки/колонки/ANSI padding
  живого кадра. Canonical history/search/reader не меняются; mapping сохраняет
  выделение при переходах, замена документа откладывается на время mouse drag.
- Streaming batch RichEdit использует TOM Freeze/Unfreeze вместо временного
  снятия WS_VISIBLE. Native background меняется только при реальной смене цвета,
  обычные таймеры не запускают full repaint. Viewport восстанавливается до thaw
  и после него; затем выполняется buffered UpdateWindow.
- Главная причина стирания вывода: parent paint перекрывал WS_EX_TRANSPARENT
  RichEdit, включая NC-thumb. На реальном EXE child→parent-only→child давал
  1801→0→1801 светлых пикселей без изменения текста. `paint_shell` теперь явно
  исключает actual full rect видимых output/vt_output/input из target DC.
  После исправления 1801→1801→1801. ANSI и opacity токены preview.14 сохранены.

**Итоговые проверки:** fmt, clippy `-D warnings`, workspace tests **685 passed**
(app608 + worker3 + PTY18unit/15integration + terminal41), release build passed.
Логи `target/frosted-indigo/{fmt,clippy,tests,release}-preview15-final.log`.
Самопроверка и один независимый code-review + точечный review clipping без
подтверждённых замечаний; `git diff --check` passed. `layout.rs` не менялся.
Решение: `docs/decisions/0017-native-live-presentation.md`.

**Проверки на итоговом SHA** (пути внутри `target/frosted-indigo/`):
- `streaming-render/live-parent-clip-preview15/summary.json` — **23 passed**,
  0 failed/skipped/error. 152 экранных кадра/60 batches: статичные glyphs,
  chrome и фон побитно стабильны, visibility непрерывна, firstVisibleLine=0;
  scroll top/bottom возвращает весь journal и rail с delta=0. Без forced paints
  во время streaming, без PrintWindow/сети/LLM/global input.
- `opencode-visual/final-preview15/summary.json` — **12 passed**: replay сохранённого
  UTF-8 VT fixture в реальном EXE, ConPTY110×36/native31 строки, позиции/верх страницы
  и отдельный composer. `opencode-startup-screen.png` просмотрен. Это replay,
  не запуск настоящего OpenCode и не сетевой запрос.
- `accent-probe/menu-message/541CFD2C2B73/summary.json` — **13 passed**, native
  modal/default/cancel/keyboard/menu captions/MSAA. Private desktop, его PNG
  не объявляются экранной композицией. Все собственные окна/процессы закрыты.
- `preview15-single-input-final/run-20261031-190036-20575806/summary.json` —
  **99 passed / 16 skipped / 0 failed/error**; skips foreground/UIA caret на
  private desktop. `preview15-material-final/summary.json` — **37 passed**.
- `preview15-geometry-final/summary.json` — **260 passed / 1 skipped**, прежняя
  геометрия/native names/maximize/restore/shell. Ранние streaming candidates
  с failures сохранены как диагностика, итоговой приёмкой не являются.

NVDA, настоящий сетевой Kimi/OpenCode и физические мониторы DPI человеком
не проверялись в этой итерации. Windows-owned folder picker/system menu штатные.
Нативные hidden DPI fixtures не заменяют проверку на реальных мониторах.

**Сборка:** `target/release/bks-app.exe` и `bks-app-0.4.0-preview.15.exe`,
2 999 808 байт, SHA256
`541CFD2C2B73E733294A77644AC5CFB65CF3B72ED28FC9193CB853FEABF4AF01`.
Метаданные `target/frosted-indigo/release-preview15.json`.
Пользовательские старые экземпляры не закрывались; пакеты, commit/push не делались.
Все прежние незакоммиченные TUI-изменения сохранены.

### Предыдущий результат — 31.10.2026, 0.4.0-preview.14

По скриншоту пользователя исправлена классическая голубая шапка и светлая
правая рамка, появлявшиеся поверх стекла. В собственной native fixture
воспроизведены два источника: WM_SETICON(big) и WM_NCACTIVATE(FALSE), даже
при прежнем DefWindowProc(..., -1). `window_frame::custom_frame_message`
подавляет NC/UAH painting; неминимизированный WM_NCACTIVATE возвращает TRUE;
SETTEXT/SETICON проходят штатную обработку под кратким redraw lock видимого
окна. Native title/icon, исходная видимость и чужой redraw lock сохранены.
Hit testing, WS_OVERLAPPEDWINDOW и расположение элементов не менялись.

Стекло облегчено: общий Acrylic tint 144→128, фон терминала/ввода 184→144,
боковая панель 136→112, нижняя полоса 90→80 (alpha из 255). Буквы, selection
и ANSI-фоны непрозрачны. Экранный отклик вывода на смену цветного фона вырос
примерно в 1,8 раза. На белом окружении измерены контрасты: заголовок 4,906:1,
основной терминальный текст 6,360:1; UI-пиктограммы ≥3:1.

**Проверки:** fmt, clippy `-D warnings`, workspace tests **671 passed**
(app594 + worker3 + PTY18unit/15integration + terminal41), release build passed.
Добавлен native regression сохранения title/icon/visibility/redraw lock.
Логи: `target/frosted-indigo/{fmt,clippy,tests,release}-preview14-final.log`.
Независимое ревью подтверждённых замечаний не нашло; `git diff --check` passed.

**EXE/visual:** `preview14-material/summary.json` — 37 passed, без failed/skipped;
`preview14-contrast/summary.json` — 37 passed и измерения в `contrast.json`;
`preview14-geometry/summary.json` — 260 passed / 1 skipped / 0 failed.
Все каталоги внутри `target/frosted-indigo/`. Реальные экранные снимки просмотрены.
`accent-probe/exe-frame/83ACC93FA72D/live/summary.json` — 8 сценариев на этом EXE,
0 изменений пикселей caption/правого края после SETICON, SETTEXT, synthetic
NCACTIVATE, max hover, resize, выхода влево и возврата. `private/summary.json`
подтвердил native maximize/restore; его PrintWindow не считается экранной
композицией. Реальная смена foreground, NVDA и физические DPI остаются ручными.

**Сборка:** `target/release/bks-app.exe` и `bks-app-0.4.0-preview.14.exe`,
2 948 096 байт, SHA256
`83ACC93FA72DFF6FBEC1D00775B16C81757A41638CD7EC2CCBE075DADA70234F`.
Метаданные — `target/frosted-indigo/release-preview14.json`.
Предыдущий пользовательский экземпляр preview.13 не закрывался и не заменялся.
Пакеты не создавались, commit/push не выполнялись. Чужие TUI-правки сохранены.

### Предыдущий результат — 31.10.2026, 0.4.0-preview.13

Выполнен редизайн по PNG с сохранением расположения элементов. Последнее
уточнение пользователя важнее исходных индиговых токенов спецификации:
нейтральное серое стекло по **всему** окну; оттенки приходят от фона.
Матовый вывод и поле команды теперь тоже пропускают размытый фон, буквы,
выделение и ANSI остаются непрозрачными. Коммита/push нет; все прежние
незакоммиченные TUI-изменения сохранены. `layout.rs` не менялся.

- Новая тема, Segoe UI Variable Text/Display с fallback, Cascadia Mono 17 DIP,
  векторные иконки профилей и многоразмерный ICO, вкладки и статусные капсулы.
- `glass_buffer`, `terminal_glass`, `backdrop`, `accent_backdrop`: нативная
  alpha-композиция, настройки отключения, High Contrast/opaque fallback.
  Accent4 + `DWMNCRP_DISABLED` сохраняют эффект inactive; WM_NCPAINT/WM_NCACTIVATE
  подавляют классическую заливку заголовка, стандартные оконные стили сохранены.
  Простые перестановки DWM-вызовов и AccentFlags проблему не решали.
- Исправлен подтверждённый сброс RichEdit-цветов после темы/шрифта: кэш формата
  инвалидируется, native regression проверяет реальные цвета/выделение/текст.
- Архитектурное решение и ограничения: `docs/decisions/0016-native-frosted-glass.md`.

**Итоговые проверки:** fmt, clippy `-D warnings`, workspace tests — **670 passed**
(app593 + worker3 + PTY18unit/15integration + terminal41), release build passed.
Логи: `target/frosted-indigo/{fmt,clippy,tests,release}-final.log`.
`git diff --check` passed. Исходные PNG/MD референса не редактировались.

**Native/visual:** `whole-glass-acceptance/summary.json` — **37 passed**, ноль
failed/skipped/error: экранное A/B всех зон, включая вывод и ввод, при настоящем
inactive HWND; отключение даёт стабильную непрозрачную поверхность.
`whole-glass-geometry/summary.json` — **260 passed / 1 skipped**, геометрия
относительно baseline, resize/maximize/restore, native names и shell output.
`terminal-glass/final-interactions/summary.json` — **18 passed / 1 skipped**:
scroll top/bottom и selection/clear возвращают точно те же пиксели; skipped
caret blink без foreground. Эти каталоги внутри `target/frosted-indigo/`.
`single-input-final/run-20261031-165114-3b232e5c/summary.json` — **99 passed /
16 skipped / 0 failed/error** на итоговом EXE: shell/TUI, общий ввод, Unicode,
native menu, source echo; skips foreground/UIA caret на private desktop.
Ранние material-диагностики имеют skips/failures и не являются итоговой приёмкой.
Большой пробел между startup warning и prompt в отдельных запусках — пустые
строки самого PTY-журнала; воспроизведён и в fresh opaque процессе, не новая
геометрия RichEdit. NVDA и реальные 125/150/200% мониторов не объявлены passed.

**EXE:** `target/release/bks-app.exe` и `bks-app-0.4.0-preview.13.exe`,
2 947 584 байта, SHA256
`AF13A337660309B5FB0CC54278F69E1B192218AA09C9E7089B750247D78D1FD9`.
`target/frosted-indigo/release.json` содержит те же метаданные. Пакеты preview.13
не создавались; существующие preview.12 в `target/packages` не заменялись.
Следующий шаг — пользовательская визуальная приёмка и NVDA, без повторного
исследования материалов или полного круга ревью.

### Предыдущий результат — 31.10.2026, 0.4.0-preview.12

Пользователь возобновил вчерашнюю работу. Незавершённая preview.12 доведена
до release EXE и пакетов; остановка 30.10 больше не действует. Коммита/push нет.
Следующий шаг — ручная приёмка этой сборки с NVDA, без повторного исследования
и полного круга ревью. Все накопленные незакоммиченные изменения сохранены.

**Результат:**

- Закрыт оставшийся M в `tui_suggestions.rs`: default foreground не считается
  обычным цветом автоматически. Два равноправных цветовых варианта дают None;
  DIM и неизменные строки списка определяют палитру. Новая регрессия сначала
  воспроизвела ошибочный выбор, затем прошла после исправления. Проверены оба
  направления, wrap, разные цветовые ориентации, два DIM-пункта и списки 3+.
  Все 25 тестов меню passed. Логи `target/tui-preview12-menu-{before,after}.log`.
- В итоговую сборку вошли вчерашние исправления: ответы перед неподвижным
  footer, busy по подсказке отмены и подтверждённая готовность после её снятия,
  отдельные одинаковые запросы после свежего echo, сохранение повторяющихся
  абзацев, речь без повторов при переносах, Panel/gutter composer и NBSP,
  source-confirmed ввод/короткая bracketed paste, temporal foreground/DIM меню,
  потребление намерения фокуса первым новым кадром вместо позднего popup.
- README/F1/help/CHANGELOG/manual/known-limitations согласованы, ADR0015
  дополнен контрактом Panel/evidence/temporal menu и неоднозначного выбора.
  Никаких адаптеров по имени программы или автоматического исполнения нет.

**Конечные обязательные проверки passed:**

`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace` — **658 passed** (app581 + worker3 + PTY18unit/
15integration + terminal41), `cargo build --release`.
Логи `target/tui-preview12-{fmt,clippy,tests-final,release-final}.log`.
Самопроверка исправления и diff выполнена; `git diff --check` passed с учётом CRLF.
Вчерашний независимый review уже завершён; новый полный review не запускался.

Первый workspace run: 578 app passed / 3 native RichEdit failed (selection /
replace), `target/tui-preview12-tests.log`. Без правок RichEdit отдельные 7
native tests прошли, затем повторный и окончательный workspace runs прошли.
Причина нестабильности не установлена; её исправление не заявлено. Логи
`target/tui-preview12-richtext-isolated.log`, `target/tui-preview12-tests-repeat.log`.

**Native итогового EXE:**

- `-TuiTurns`: **216 passed / 3 skipped / 0 failed / 0 error**,
  `target/preview12-tui-turns/run-20261031-122520-933224db/summary.json`.
  Новый bullet case проверил fixed footer, italic reasoning, styled body,
  отсутствие Ready при `ctrl+c: cancel`, завершение без Status:done,
  два одинаковых запроса, ED2, body-only nav, reader selection/viewport/focus,
  сохранение цветов и точные байты `2F0D710D710D710D710D`.
- `-SingleInput`: **99 passed / 16 skipped / 0 failed / 0 error**,
  `target/preview12-single-input/run-20261031-122520-12da6f5d/summary.json`.
  Одно поле, Unicode/source echo, native menu, rule composer и shell regressions.
- Skipped: foreground auto-focus и interactive EDIT UIA caret/selection
  недоступны на private desktop; native EM_GETSEL/UIA text/styles проверены.
  Речь NVDA, ready sound и человеческая приёмка не объявлены passed.
  Скриншоты bullet live/archive и rule-menu просмотрены.
  Bootstrap cleanup обоих сценариев: см. `private-desktop-bootstrap.json`;
  собственные тестовые окна закрыты, пользовательские сессии не менялись.

**Сборка:** `target/release/bks-app.exe` и
`target/release/bks-app-0.4.0-preview.12.exe`, **2692608 bytes**, SHA256
`5E747B02CD16AB9C81D8E2F7BCFE493D1E3EAF3ACDCD70B7FC11E5DF25E0C66F`.
Versioned EXE — точная копия проверенного release. Portable/setup/manifest/
SHA256SUMS — `target/packages/bks-term-0.4.0-preview.12-windows-x64-*`.
Extraction/hash passed; private package install/update/uninstall **48 passed /
1 skipped** (file-symlink, Windows1314), без реестра/ярлыков пользователя.
Логи `target/tui-preview12-package{,-private}.log`.
Для Windows PowerShell5.1 при вызове `scripts/package.ps1` явно задавать
`-Exe` и `-OutputDirectory`: default с PSScriptRoot не вычисляется при `-File`.
Новая пользовательская preview.12 не запускалась; прежние пакеты сохранены.

**Границы совместимости:** вчерашний actual private OpenCode2.0.32 был проверен
без credentials/model requests (`target/opencode-diagnosis/`, public fixture
`crates/bks-app/tests/fixtures/gutter-panel-menu.vt`). Claude2.1.240 проверен
по установленному renderer/source-equivalent VT; private startup/--ax дошёл
до theme/trust, доверие/логин не подтверждались (`target/claude-tui-diagnosis/`).
Авторизованный Claude REPL не проверен. Большая/первая многострочная вставка
может сохранять raw fallback; full projection не заявлена. Вручную проверить
Kimi Ready/речь стадий и навигацию ответов, поля/меню Claude и OpenCode:
`docs/manual-final.md`. Старые ExecutionPolicy/auth причины не объясняют
исправляемые поля/меню. Не читать/не менять `docs/note.txt` и пользовательские
новые design reference файлы. Коммиты/push — только по явному разрешению.

### Предыдущий результат — 30.10.2026, 0.4.0-preview.11

**Запрос:** выполнить четыре согласованных пункта: сигнал готовности ответа,
навигация между ответами, речь видимых размышлений/поиска/инструментов,
исправление смещений графических символов. Дополнительно пользователь
показал неподдержанное поле Claude и запрет запуска opencode.ps1.
Реализация общая для TUI, без распознавания имени программы. ADR0015.

**Сделано:** `tui_turns.rs` наблюдает канонические ячейки, стили и live-проекцию.
Кандидат запроса возникает после принятого явного Enter; меню, защищённый ввод,
пустой Enter и slash-команды исключены. `tui_submission.rs` сохраняет bounded
evidence принятых байтов до VT echo, включая partial ack/hidden cursor;
это только подсказка классификации, отображаемое поле остаётся source-owned.
Неизвестные изменения не угадываются и не становятся исполняемыми действиями.

Готовность требует ответа, свежего завершения либо реального unavailable→editable
после busy, 150ms стабильности текста и 400ms границы. Постоянно доступное поле,
пауза и старый done этого не доказывают. Ошибка, прерывание и ожидание действия
различаются; неподдержанный формат даёт NewOutput без выдуманной готовности.
Footer берётся из live-области; fenced/quoted status и старый scrollback не
подменяют состояние. Старый статус должен исчезнуть, чтобы стать свежим.

Журнал содержит только тела ответов с оформлением, до128 записей/2МиБ,
stable IDs, один ответ при streaming/corrections/ED2. Full snapshot corrections
заменяют тело, доказанный overlap дописывает хвост; user-like строки внутри
code fences сохраняются. Завершённые ответы неизменны. Перекрывающиеся запросы
без новой доказанной границы не разделяются автоматически. Журнал в памяти
вкладки, явная очистка через меню; shell clear его автоматически не удаляет.
Ctrl+Up/Down — соседние ответы, Ctrl+Shift+R — последний; неизвестный TUI
сохраняет логические строки/новый вывод. Архив и live используют тот же1018,
с отдельными ReadingState и ключом view/revision; поиск учитывает источник.
Esc/возврат к вводу восстанавливает live; пассивный ответ не меняет focus,
selection или viewport читаемого ответа.

`turn_speech.rs` — единая ограниченная очередь известных и неизвестных TUI
событий, без второго raw All channel. Озвучиваются новые стабильные видимые
шаги; static thinking:high/spinners не повторяются. Ready не переигрывает тело
через passive NewOutput. Фоновые вкладки/окно не накапливают речь, набор
сокращает pending ready до короткого сообщения. UIA progress использует
CurrentThenMostRecent; STATIC1031 показывает состояние, не меняя фокус.
Отдельные settings `answer_ready_sound` и `auto_read_answers` по умолчанию false;
новые options есть в нативных настройках и сохраняются при multiwindow merge.
Режим Manual отключает речь, явно включённый звук остаётся независимым.

`terminal_metrics.rs` измеряет фактический resolved RichEdit font и его insets
с постоянным резервом scrollbar. ConPTY использует исходную область без
изменения размера при поиске/списке. TOM2 CharRep/face и проверенный spacing
исправляют BOX/BLOCK и наследование symbol font пробелами, сохраняя wide,
combining, emoji, цвета, readonly и закладки. AUTOFONT целиком не отключается.
Live Grid и архив Reflow сохраняют native wrap: объединённый VT soft-wrap
абзац виден полностью и копируется без добавленных переносов. В тесте tail
одного абзаца доступен вертикальной прокруткой через47 visual lines.

**Claude/OpenCode:** общий composer дополнен matching horizontal rules и
prompt справа, включая wrap, hidden cursor и history proofs. Новый native
rule/menu case подтверждает одно поле и точные bytes, без Claude adapter.
Установленный PowerShell7 предпочтителен при discovery; явные/сохранённые
профили WindowsPowerShell5 и аргументы сохранены. Добавлен CLI OpenCode:
нативный npm EXE предпочтителен, safe CMD fallback, bare/cmd/exe handoff.
Локальная диагностика: WindowsPS5.1 Restricted, pwsh7.6.9 RemoteSigned;
bare opencode разрешается сначала в npm PS1. Политика/реестр/учётные данные
не менялись; в явном PS5 можно использовать opencode.cmd. Login expired
на экране Claude требует входа средствами самого Claude.

**Конечные проверки passed:** fmt, clippy workspace/all-targets/-D warnings,
**615 tests** (app538 + worker3 + PTY18unit/15integration + terminal41), release.
Логи `target/turns-{fmt,clippy,test,build}-final.log`.
Один независимый review и адресная проверка исправлений завершены: footer
freshness/availability, fenced content, snapshot replacement, быстрый Enter,
unknown speech после готового ответа исправлены и покрыты регрессиями.
Нативные font fixtures со шрифтами Theme96/144 —6passed; их lifecycle
сериализован только в тестах после воспроизводимой нестабильности при
конкурентной калибровке. Внутренний механизм RichEdit не установлен;
продукт использует один UI thread. Строгие first-pass/same-text-sync widths
90/130px сохранены; три повторных metrics run и полный app passed.
Логи `target/terminal-metrics-probe/`; direct-pipeline incomplete/error log
не является доказанным AV или passed, окончательная проверка — через Cargo.

**Native итогового versioned EXE:**

- TuiTurns: **110passed/3skipped/0failed/error**,
  `target/tui-turns-verified/run-20261030-210658-fb5afe34/summary.json`.
- SingleInput с новым rule/menu case: **99passed/16skipped/0failed/error**,
  `target/turns-single-input-verified/run-20261030-210658-73feda85/summary.json`.

Проверены два styled ответа после ED2, body-only nav и UTF16 positions,
stable busy без ложного Ready, сохранение reader selection/viewport/focus,
один EDIT, source-owned slash selection, точные bytes, старые shell regressions.
Root просмотрел final archive/rule-menu PNG. Skipped: interactive EDIT UIA
caret/selection и foreground auto-focus на private desktop; реальная речь,
звук и NVDA не объявлены passed. Native EM_GETSEL, UIA text/styles и LISTBOX
selection проверяются строго. Bootstrap обоих сценариев: Default восстановлен,
own desktop/job закрыты, ChildExitCode0, Failure=null. Реальные сеансы моделей
и авторизация этими сценариями не проверяются.

**Сборка:** `target/release/bks-app.exe` и `bks-app-0.4.0-preview.11.exe`,
2610688bytes, SHA256 `01528FEBF93CD2C50AFE0CE9ABC51BB751FC976CEB8B0DA633E4AED1283B8EB2`.
Portable/setup/manifest/checksums в `target/packages`; extraction/hash passed.
Private install/update/uninstall **48passed/1skipped** (file-symlink1314),
логи `target/turns-package{,-private}-final.log`. Настоящая установка, реестр
и ярлыки пользователя не изменялись. README/F1/help/CHANGELOG/manual/ADR обновлены.
Проверочные окна закрыты. Итоговый EXE для пользователя ещё не запускался;
прежняя явно запущенная preview.10 сессия не закрывалась (исторический PID3912,
не использовать без повторной проверки). Коммита/push нет.

**Дальше:** ручная проверка нескольких TUI и реальной речи NVDA: готовность,
размышления/поиск/tool progress, навигация и цвета архива, звук/full answer
options, неизвестный вывод после известного, авторизованный Claude и OpenCode.
Настоящий WM_DPICHANGED/переход между мониторами и фактическое выравнивание
рамок проверяются человеком; тесты Theme96/144 этого не заменяют.

### Предыдущий результат — 30.10.2026, 0.4.0-preview.10

**Запрос:** пользователь подтвердил ls/clear preview.9 и поручил общее поле:
в shell редактировать команду, в TUI отражать ввод приложения; перенесённые
поле и рамка не должны дублироваться в верхнем выводе. Решение общее,
без привязки к Kimi или другой программе. Архитектура — ADR0014, заменяет
часть ADR0012 о canvas/companion focus.

**Сделано:** сохраняется один EDIT1002, shell editable / TUI readonly mirror.
В TUI явные клавиши идут приложению, текст и UTF16-каретка подтверждаются VT.
Собственные slash/autocomplete не исполняются по распознанному тексту.
Строго найденные поле/рамка и меню исключаются из styled VT reader1018;
canvas1017/companion1011 скрыты. ComposerHistory исключает подтверждённые
исторические строки по absolute anchors и fingerprint, включая partial scroll.
Неоднозначный экран остаётся целиком, поле — пустым прямым вводом. Geometry,
alternate/history reset или предел10000 строк сбрасывают неоднозначные proofs.
Скрытый ввод остаётся redacted, side-by-side body не удаляется.

Подсказки — native LISTBOX1030 возле того же поля; selection только из VT,
без native typeahead и второго custom menu speech. Source focus требует
явного принятого ввода и нового VT кадра; foreground gate сохранён.
Shell draft + selection отдельны от TUI. View epoch отделён от raw VTrev:
поиск/reader rebasing обновляются при смене режима даже без новых PTY bytes.
Поиск и popup не меняют ConPTY dimensions. DirectUiSync защищает native
focus callbacks при скрытии списка. Same-HWND handoff восстанавливает Tab
capture; Ctrl+A идёт TUI, локальное Shift-выделение остаётся для копирования.
Hidden cursor при неизменном поле сохраняет локальное выделение; возврат
того же подтверждённого курсора его не сбрасывает. Send возвращает field focus.
Выход в shell возвращает editable field, draft и journal. Stale Enter не может
пересечь handoff по MSG.time/GetTickCount; это также имеет pure regression.

**Проверки passed:** fmt, clippy workspace/all-targets/-D warnings,
**572 tests** (app495 + worker3 + PTY18unit/15integration + terminal41), release.
Логи `target/one-input-{fmt,clippy,test-final,build-final}.log`.
Независимый review core выполнен, подтверждённые замечания исправлены;
поздние focus/capture fixes просмотрены адресно, final diff-check passed.
Ранний full test однажды завершился AV; повторить его не удалось (отдельные
native/full runs и конечные workspace runs passed). Причина не установлена,
COM teardown вслепую не менялся; диагностика `target/tui-suggestions-crash/`.

**Native конечного EXE:** `-SingleInput`, **82passed/12skipped/0failed/error**,
`target/one-input-verified/run-20261030-185332-31160ef6/summary.json`.
Проверены Unicode/multiline, source-ack text/caret, initial Tab, held keys,
menu/list semantics/exact bytes, reader/search stale events, masked/ambiguous
fallback, hidden-cursor copy selection, manual/automatic shell return и прежние
output-follow/passive-history/split-clear regressions. Root просмотрел конечные
multiline/menu PNG: одно поле, рамки/копии нет, кнопка «Отправить».
12skipped: EDIT UIA selection/caret и automatic foreground focus недоступны
на private desktop; human NVDA и end-to-end stale-Enter race не объявлены passed.
Native EM_GETSEL, UIA text, LISTBOX selection и byte ledger проверяются строго.

`check-unified-tui.ps1 -SingleInput` теперь запускает fresh PS/BKS/UIA worker
на уникальном private Win32 desktop без SwitchDesktop/physical/global keys/
clipboard/real CLI/network. Создатель восстанавливает Default, закрывает own
job/desktop; final bootstrap receipt подтверждает это. Logical focus fallback
использует только own WM_COMMAND и не ослабляет production foreground gate.
Default/Minimal/ReadNavigation/TuiSelection/PaintIsolation/CliHandoff — старый
ADR0012 canvas contract для preview.8/9; текущая TUI приёмка — SingleInput.

**Сборка:** `target/release/bks-app.exe` и versioned preview.10,
2449920bytes, SHA256 `2835072F9E9D05132CF6F0CE9AA8EAF14B31E7151880905B1EB9563E6C559BCE`.
Portable/setup/manifest/SHA256SUMS в `target/packages`; extraction/hash passed.
Private install/update/uninstall48passed/1skipped(file-symlink1314), настоящие
registry/shortcuts/установка пользователя не изменялись. Логи
`target/one-input-package{,-private}.log`. README/help/CHANGELOG/manual/ADR обновлены.
Проверочные окна/фикстуры закрыты; пользовательская сессия preview.10 не запущена.
Старые PID ниже не использовать без повторной проверки. Коммита/push нет.

**Дальше:** ручная NVDA-проверка общего поля/меню на нескольких TUI, редактирование
(Ctrl+A/Home/End/Backspace/Delete/paste), речь без повторных slash/рамок,
чтение цветов и возврат к shell. Не считать UIA-автотест фактической речью NVDA.

### Предыдущий результат — 30.10.2026, 0.4.0-preview.9

**Ручная проверка preview.8:** пользователь подтвердил: «цвет есть, всё
озвучивается нормально». Следующий запрос — обычные команды: `ls` выполняется,
но новый результат не прокручивается; `clear` не очищает видимый терминал.
Эта итерация исправляет показ результатов и очистку журнала оболочки.
Коммит/push не разрешались и не выполнялись; `docs/note.txt` не менялся.

**Прокрутка:** успешная явная отправка команды показывает журнал и EOF,
сохраняя фокус в поле «Команда». Если прежде читали VT, его позиция сохраняется
перед переходом к результату. Пассивный вывод при историческом выделении/
прокрутке остаётся на месте. Нативный hidden RichEdit test доказал две проблемы:
EM_SCROLLCARET без фокуса не двигает журнал; SB_BOTTOM ставит его на один пиксель
до inclusive scrollbar boundary. Теперь follow использует SB_BOTTOM, bottom
check допускает1px, selection/viewport восстанавливаются после redraw resume.
Один native test проверяет EOF-follow и сохранение history view после append.

**Очистка:** TerminalOutput выдаёт Clear(UTF16 offset) для настоящего полного
ED2/ED3. В обычной оболочке Session отсекает preclear text/events/styles,
очищает старые блоки, selection/viewport/trim/prompt capture/speech state,
сохраняет postclear вывод, настройки/draft/parent identity/PTY. Main потребляет
take_journal_reset даже при отсутствии printable text, синхронизирует final model
и пропускает старые chunks того же poll; retention trim остаётся отдельным.
Начальный ConPTY ED2 — layout: он не стирает блок команды, принятой до первого
вывода CMD. Direct/autoCLI/CLI-profile и alt47/1047/1049 не очищают журнал.
Пустой ConPTY обход строк распознаётся только как bounded K/CRLF + HOME,
обычные postclear переводы строк не отрезаются. Имена clear/cls не перехватываются.
Собственный PowerShell/ConPTY подтвердил Clear-Host/cls/clear с exit0;
реальный поток здесь содержит обход пустых строк и CSI3J. Архитектура — ADR0013.

**Проверки passed:** fmt, clippy(workspace/all-targets,-D warnings),
**538tests** (app461+worker3+PTY18unit/15integration+terminal41), release.
Логи `target/shell-output-{fmt,clippy,test,build}.log`; targeted parser30 и
session39 passed, actual clear probe — `target/shell-output-clear-probe.log`.
Независимый code-review approved, значимых дефектов в финальном коде не нашёл.

**Native на конечном EXE:** расширенный CliHandoff38passed/0failed/skipped/error,
`target/shell-output-handoff/run-20261030-151735-76fc87ba/summary.json`.
Сохранены старые handoff/reader/canvas проверки; добавлены явный submit из
истории→EOF, продолжение follow, passive history selection/viewport, split
clear без текста→пустой native/UIA journal→ровно один новый prompt. Actual EOF7872,
bottom5362/5363; passive selection3744..3768, firstline147, scroll2678 сохранены.
Raw input47bytes — четыре явные literal fixture команды, лишних байтов нет.
PaintIsolation22passed/1skipped/0failed/error,
`target/shell-output-paint/run-20261030-151744-970ac08b/summary.json`.
Native patch изменил0px, VT positive145px. Cleanup собственных PID9744/18032
и19084/18440 passed без force. Root просмотрел shell-output-follow и
shell-clear-prompt PNG. Human NVDA/воспринимаемое мерцание skipped.

**Сборка:** `target/release/bks-app.exe` и versioned preview.9,
2353664bytes,SHA256 `2B471BB3DA5A8F715CB6A82543029E115B6BBEA94F9C875FC0DDA6F4F701168B`.
Portable/setup/manifest/SHA256SUMS в `target/packages`; extraction/hash passed,
private install/update/uninstall48passed/1skipped(file-symlink1314), без реальной
registry/shortcuts/установки пользователя. Логи `target/shell-output-package-{build,private}.log`.
README/справка/CHANGELOG/manual обновлены на preview.9.

**Открыто пользовательское окно:** preview.9 PID18568, HWND6095816,
`target/release/bks-app-0.4.0-preview.9.exe`, cwd проекта; process-local launcher
очистил только flags цвета среды агента, глобальная среда не менялась.
Metadata `target/shell-output-user-launch.json`. Старые пользовательские окна
не закрывались и команды в них не отправлялись. Новое окно не закрывать/
не вводить автоматически; перед действиями перепроверять PID/HWND.

**Дальше:** пользователь проверяет в новом окне `ls`, затем `clear`/`cls`/
`Clear-Host` и повторный вывод. Положительное подтверждение цвета/речи относится
к preview.8; NVDA-приёмка новых scroll/clear изменений остаётся ручной.
Ранее обсуждалось неудобство отдельного переключения для чтения формата;
фокусная архитектура ADR0012 этой итерацией не пересматривалась.

### Предыдущий результат — 30.10.2026, 0.4.0-preview.8

**Ручной результат preview.7:** пользователь показал речь NVDA с лишними `/`,
`>` и `)` при навигации slash/model; содержательная речь выбора уже есть.
После `/exit` Kimi остался прямой ввод, хотя PowerShell prompt вернулся.
Уточнено человеком: `kimi` набирался именно в прямом вводе; цветов нет и
визуально, и в чтении NVDA. Разрешено исправление этих трёх проблем; commit/push
не разрешались и не выполнялись. Посторонний `docs/note.txt` не менялся.

**Фокус и речь:** raw TUI теперь фокусирует существующий BksTerminalSurface
canvas1017 с TABSTOP и без TextPattern редактора. RichEdit1011 остаётся
readonly styled companion за canvas, без TABSTOP; его raw focus перенаправляется
на canvas. Это устраняет конфликт с NVDA EditableText caret movement scripts,
которые дополнительно читают текст у каретки после стрелок. Ctrl+Shift+Space
и Shift+навигация по-прежнему открывают reader1018 с обычным выделением и
форматом; Esc возвращает TUI. Mode/capture, stale Enter, Tab и mouse focus
адаптированы. Архитектура и первоисточники механизма NVDA — ADR0012.
Самостоятельные `/`/`)` не проходят custom InputFeedback по его контракту;
из содержательных labels пунктуация не удаляется вслепую.

**Меню:** перенос selector теперь сопоставляет уникальные стабильные labels
при перемещении viewport, а не только на одинаковых физических строках.
Realistic swarm/auto/effort и model/provider fixtures сохраняют аргументы
`[on|off] | <task>` и описание. Duplicate labels/неоднозначность остаются тихими.

**Возврат shell:** native submit_command уже arm-ил auto_cli_input, raw write
этого не делал — именно такой путь подтвердил пользователь. Теперь main
вызывает record_direct_input только после принятого write: bounded UTF8
capture до4096байт, простой ASCII Backspace; неизвестное редактирование,
history/escape/control/неполный ввод не восстанавливаются по VT-тексту.
Enter потребляет старую готовность; CLI arm требует точной введённой команды.
CR/CRLF допустим только в конце accepted batch, multiline хвост не arm-ит CLI.
Возврат требует свежего совпадающего OSC133 parent prompt, не строки Bye/PS.
Обычная raw shell command сохраняет ручной direct mode. Настоящий PowerShell
с собственной local function kimi (без запуска установленного CLI) проверил
native/raw пути, alt-screen cleanup и resize. Повторных байтов не отправляется.

**Цвета:** причина подтверждена из статического installed Kimi bundle и own
ConPTY probe: lpEnvironment=None наследовал TERM=dumb от среды агента,
Chalk5.6.2 выдавал level0 при isTTY=true. Новый terminal_environment в bks-pty
передаёт отдельный Unicode block: TERM=xterm-256color, COLORTERM=truecolor,
TERM_PROGRAM=bks-term; чужой TERM_PROGRAM_VERSION удаляется. GetEnvironmentStringsW
сохраняет raw UTF16, drive aliases, PATH/cwd/user vars; глобального set_var нет.
NO_COLOR/FORCE_COLOR/CLICOLOR* пользователя сохраняются. Caps-only probe при
сохранённом NO_COLOR=1 подтвердил Kimi Chalk level3; реальные Kimi startup/API
и сеть для проверки не запускались. Артефакты — `target/color-audit/`.
Для NVDA текст с оформлением читается в reader через Ctrl+Shift+Space;
ручное чтение формата — NVDA+F. Это не заявление о пройденной речи NVDA.

**Проверки passed на конечном коде:** fmt, clippy(workspace/all-targets,-D warnings),
**528tests** (app451+worker3+PTY18unit/15integration+terminal41), release.
Логи `target/tui-feedback-{fmt,clippy,test,build}.log`. Один независимый проход
focus/session/menu не нашёл новых значимых дефектов; self-review нашёл accepted
multiline tail, он исправлен и проверен targeted reviewer без нового полного круга.

**Native конечного EXE:** CliHandoff20passed/0failed/skipped/error,
`target/tui-feedback-handoff/run-20261030-143030-30ccd9dd/summary.json`;
PaintIsolation22passed/1skipped/0failed/error,
`target/tui-feedback-paint/run-20261030-143039-08997b15/summary.json`.
Проверены actual canvas focus, canvas без TextPattern/companion с полным readonly
TextPattern, TABSTOP, clipping/z-order, reader state и отсутствие лишнего PTY.
Native patch изменил0/663552px, VT positive145px; root просмотрел итоговый PNG.
Cleanup own app/fixture PID17204/15984 и11568/12244 passed без force.
Human NVDA/flicker skipped. Старые клавиатурные ветки harness по-прежнему
ожидают focus1011; их не запускали и их успех не заявлялся.

**Сборка:** `target/release/bks-app.exe` и versioned preview.8,
2362368bytes,SHA256 `A0473F7A2AF449E425352622BF6677B150463BECEDE0EC2E42DFBA6BE0611D51`.
Portable/setup/manifest/SHA256SUMS — `target/packages`; extraction/hash checks
passed, private package install/update/uninstall48passed/1skipped(file-symlink1314).
Логи `target/tui-feedback-package-{build,private}.log`; реальная registry/
shortcuts/чистая VM не проверялись. README/справка/CHANGELOG/manual обновлены.

**Открыто новое пользовательское окно:** preview.8 PID7912, HWND722938,
`target/release/bks-app-0.4.0-preview.8.exe`, cwd проекта. Сняты только известные
color flags среды агента в окружении этого нового процесса; системные настройки
не менялись. Metadata `target/tui-feedback-user-launch.json`. Старый пользовательский
PID4544 исчез до нового запуска; агент его не закрывал. Новое окно не закрывать,
не вводить автоматически команды и перепроверять PID/HWND перед любыми действиями.

**Следующее:** вручную в новом окне запустить Kimi, проверить `/` и `/model`
без дополнительной речи маркеров, `/exit`→поле «Команда», цвета нового вывода,
Ctrl+Shift+Space/NVDA+F, выделение/возврат и фактическое мерцание.
Реальную речь/пользовательскую приёмку preview.8 пока не объявлять пройденными.

### Предыдущий результат — 30.10.2026, 0.4.0-preview.7

**Поручение:** пользователь разрешил реализовать все семь согласованных
доработок TUI и передачи цветов; следующая стадия — ручное тестирование.
Все семь пунктов реализованы в коде. Коммит/push этой итерации не разрешались
и не выполнялись. `docs/note.txt` — посторонняя локальная заметка, не менялась.

1. **CSI2026:** idle poll обслуживает таймаут synchronized update без новых
   байтов, публикует ревизию и протокольные ответы один раз. Обычный END_SYNC
   сохраняет атомарность кадра; действие по снимку блокируется до публикации.
2. **Стили модели:** ReadingDocument экспортирует coalesced UTF-16 TextStyle
   spans: fg/bg, bold, dim, italic, underline, strikeout, inverse. Общий resolver
   цвета используется renderer и доступным текстом. HIDDEN редактируется до
   проекций, его исходный текст/стиль не попадают в доступные документы.
3. **Нативная доступность:** журнал1001, прямой ввод1011 и VT reader1018
   используют readonly RICHEDIT50W; поле команды1002 остаётся EDIT. Новый
   accessible_text явно переводит canonical CRLF/UTF-16 в native CP, сохраняет
   selection/scroll/event mask при форматировании. TOM font setters на readonly
   отвергаются, поэтому используется EM_SETCHARFORMAT без снятия ES_READONLY.
   Проверены CRLF между порциями, Unicode, bare CR и WM_GETTEXT expansion;
   приложение использует длину модели. Архитектура — ADR0011.
4. **Журнал:** потоковый StyleTracker использует VT parser для SGR, включая
   16/256/True Color и colon forms; OSC не накапливается повторно. Стили
   ребазируются с retention, служебные notices не меняют состояние SGR.
5. **Меню и диагностика:** подтверждается reciprocal перенос bold/underline
   дополнительно к fg/bg/inverse/marker/cursor. BKS_TERM_TUI_TRACE явно задаёт
   JSONL-путь: metadata без текста/labels/клавиш, ring64, очередь8, файл до1МиБ,
   запись вне UI thread. Причина реального прежнего молчания `/model` без
   воспроизведения не объявлена доказанной.
6. **Редактирование и чтение:** Backspace/Delete и Left/Right/Home/End дают
   подсказку только после точного видимого результата. Generic menu feedback
   отменяется после подтверждённого edit, а не при самом нажатии. Усилены
   CRLF/Unicode snap, независимое выделение при stream и неоднозначный reply anchor.
7. **Обновления:** неизменный direct document и feedback без pending действия
   не строят лишние снимки; native formatting применяется к изменённым диапазонам.
   Append расширяет source/map без полной копии; HWND cleanup освобождает кеш.
   Shared native harness переведён на координаты RichEdit, input остаётся EDIT.

**Проверки passed:** `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace` — **510tests** (app437+worker3+PTY15unit/14integration
+terminal41), `cargo build --release`. Логи — `target/tui-colors-{fmt,clippy,test,build}.log`.
В app входят шесть реальных native-тестов RichEdit/TOM/UIA на собственных скрытых
окнах: цвета TextPattern, readonly, Unicode, CRLF/patch/append, bare CR, selection,
style-only delta и lifecycle. Чтение UIA идёт отдельным MTA с owner message pump.
Независимый проход ревью нашёл конфликт edit/menu feedback; он исправлен и
покрыт двумя интеграционными регрессиями. Адресованная проверка bare CR закрыта.

**Native на конечном EXE:** CliHandoff14passed/0failed/skipped/error,
`target/tui-colors-handoff/run-20261030-125712-e2e47014/summary.json`.
PaintIsolation15passed/1skipped/0failed/error,
`target/tui-colors-paint/run-20261030-125719-d3b1741d/summary.json`.
Native-only patch изменил0/663552пикселей canvas; реальный VT spinner —145.
Cleanup обоих passed без force; тестовые процессы завершены. Клавиши/focus/
activation helpers не использовались; только own PID/HWND и managed fixture.
Root просмотрел PNG предыдущего прогона той же реализации поверхности.
Skipped — человеческая оценка NVDA/воспринимаемого мерцания, не автотест дефекта.

**Сборка:** `target/release/bks-app.exe` и
`target/release/bks-app-0.4.0-preview.7.exe`,2341376bytes,SHA256
`7E556E1D3B9E5F9A939624E6BB14C4855F9B6C55CB8468F97600CE2A03652995`.
Portable/setup/manifest/SHA256SUMS в `target/packages` обновлены на этот EXE;
extraction/hash checks passed. Private install/update/uninstall —48passed,
1skipped(file-symlink Windows1314), без реальных registry/shortcuts/запуска
пользовательского приложения. Логи `target/tui-colors-package-{build,private}.log`.

**Уточнения проверки:** промежуточный прогон дважды упал на cleanup конечного
PowerShell fixture (TerminateProcess во время нормального выхода). В тесте
добавлено bounded ожидание до2с с drain перед fallback; его содержательная
assert не ослаблена, production PTY не менялся. Целевой и полный прогоны прошли;
первый failed лог сохранён в `target/tui-colors-test-cleanup-failed.log`.
Первый setup smoke наследовал несовместимый PSModulePath PowerShell7;
повторный private запуск с process-local WindowsPowerShell module paths прошёл.
Настройки пользователя и глобальные переменные среды не менялись.

**Продолжение:** ручная приёмка preview.7 описана в `docs/manual-final.md`:
цвет/формат NVDA, удаление/курсор, `/model` и слеш-команды, чтение во время
генерации/resize/смены вкладок, High Contrast и воспринимаемое мерцание.
Ручная NVDA-приёмка этой версии не проводилась. Новых пользовательских окон
не открывали, существующие не закрывали и ввод в них не отправляли.

### Предыдущий результат — 27.10.2026, 0.4.0-preview.6

**Поручение и статус:** после preview.5 пользователь сообщил о невозможности
выделить набранный текст, лишнем чтении указателя в слеш-командах и молчании
выбора модели. Затем отложил ручную проверку, поручил обновить README,
сделать коммит и push в main. Разрешение относится к текущему BKS-Term и
`origin` (`https://github.com/BKSLab/BKS-Term.git`), веткаmain; fetch подтвердил
совпадение с53e700a перед подготовкой коммита. В него включается накопленная
реализация терминала, тесты/упаковка/CI и документация; `docs/note.txt` остаётся
локальной посторонней заметкой. Логи и сборки находятся только в ignored target.

**Выделение:** Shift+стрелки/Home/End/PageUp/PageDown из прямого ввода
открывают существующий readonly VT reader с кареткой от реального/однозначного
программного курсора либо подтверждённой вставки. Исходную клавишу с Shift/Ctrl
обрабатывает native EDIT, а не PTY. Дальше доступны обычные выделение/Ctrl+C,
Esc возвращает ввод без CLI Escape; это чтение и копирование, не редактирование
запроса CLI. Unknown cursor не угадывается. После poll проверяется foreground
и сохранение фокуса, чтобы поздний диалог не был перекрыт. Прежний Ctrl+Shift+End
остаётся глобальным ScrollToBottom. Echo, renderer и отдельное меню не менялись.

**Подсказки выбора:** доказанный перенос одного начального маркера между
устойчивыми labels озвучивает label без указателя. Статичные такие же знаки
в предыдущих prompt не подавляют этот единственный перенос; два реальных
переноса остаются неоднозначными. Без доказательства текст `> 1`, `> limit`,
`→ target` сохраняется целиком. Для цвета/инверсии поддержан перенос одинаковой
пары атрибутов на минимум двух изменившихся текстовых клетках каждой из двух
строк; неизменные provider/описание/current mark не требуют однородного стиля
всей строки. Theme/one-cell/ambiguous/HIDDEN не дают ложного выбранного пункта.
Source-derived Kimi fixtures: SelectList `→`, ModelSelector `❯` с разноцветными
именем/provider, другие передаваемые маркеры. Причина фактического runtime
молчания /model без VT trace не объявлена доказанной. Readonly диагностика
пользовательского provider извлекала только коды префиксов, без текста labels,
клавиш и дампа содержимого; встречались U+276F и U+25CF.

**Проверки passed:** fmt, clippy(workspace/all-targets,-D warnings), **467tests**
(app404+worker3+PTY15unit/14integration+terminal31), release. Логи
`target/tui-selection-speech/{fmt,clippy,test,build}.log`. Один независимый
проход нашёл P2 whole-frame marker uniqueness и недоказанное stripping;
оба исправлены, целевой reviewer подтвердил закрытие по коду и тестам.
Source-derived fixtures и автотесты не заменяют фактическую речь NVDA.

**Native проверки:** `-TuiSelection -PrepareOnly` —3passed/1skipped,
`target/preview6-tui-selection-prepare/run-20261027-175602-02806992`.
Сценарий проверяет Unicode/inverse-caret, ShiftLeft/Home/CtrlShiftLeft,
native/UIA selectedText, Esc0PTY/Tabonce, без программного Select/clipboard write.
Его actual/FG/клавиши НЕ запускались по просьбе пользователя проверить позже.
Keyless CliHandoff —14passed/0failed/skipped/error,
`target/tui-selection-speech/handoff/run-20261027-181135-df11dc6f/summary.json`.
Только два явных literal kimi CRLF в собственный fixture, raw
`6B696D690D0A6B696D690D0A`; reader selection/viewport сохранены,0extraPTY.
Собственные app15172/fixture18076 закрыты штатно и отсутствуют; FG/keys не было.

**Сборка:** 0.4.0-preview.6,2233856bytes,SHA256
`147DC16F6575A2F42197DD1AD3290FCCAA97593CD3EB8BE235AB4B4EBD6AFED3`.
`target/direct-accessibility/release/bks-app.exe`, `target/release/bks-app.exe`
и `target/release/bks-app-0.4.0-preview.6.exe` совпадают. Предыдущий default
сохранён как `target/release/bks-app.before-0.4.0-preview.6-13b715b8d5d8472da2824171902b8254.exe`.
Portable/setup/manifest/SHA256SUMS в `target/packages`; extraction/hashes passed,
private install/update/uninstall48passed/1skipped(file-symlink1314), без реальных
registry/shortcuts/пользовательского запуска. Логи package-{build,private}.log
в `target/tui-selection-speech`. README/справка/ручная приёмка обновлены.

**Продолжение:** новые пользовательские окна не открывались, существующие
не закрывались и ввод в них не отправлялся. Проверить позже реальное выделение,
речь слеш-команд/model и стабильность после preview.5; отсутствие новых жалоб
не означает приёмку мерцания. Git-результат после этого снимка смотреть по
истории main/origin, не создавать повторный коммит по одной записи памяти.

### Предыдущий результат — 27.10.2026, 0.4.0-preview.5

**Ручной результат пользователя preview.4:** «при наборе текст читается» —
озвучивание набора подтверждено человеком. Одновременно сильные рывки при
наборе и генерации ответа; пользователь отменил отдельное меню выбора/слеш-команд.
Это частичная положительная NVDA-проверка набора, не приёмка всей доступности.
Echo/direct_accessibility/announcer этой итерацией не менялись.

**Меню убрано:** удалены tui_menu.rs и tui_window/draw.rs, menu projection,
автоматический BeginControl, menu-only layout/complete и соответствующие
поля/main-тесты. Ctrl+Shift+D снова открывает прежний полный readonly обзор
с фокусом в контексте; live включается только явной кнопкой. Собственный
TUI инструмента получает `/`, выбор и обычные клавиши. Стабильные обновления
старого обзора без полного сброса списка сохранены.

**Конфликт отрисовки:** native EDIT выполнял EM_REPLACESEL/SETSEL/SCROLLCARET
с немедленными GDI/scroll операциями, а его же WM_PAINT рисовал VT. Backbuffer
не защищал от native рисования вне WM_PAINT; WM_SETREDRAW поглощался ради
WS_VISIBLE, оставляя внутреннюю native redraw включённой. Теперь существующий
canvas1017 рисует VT поверх видимого focused EDIT1011 в той же области.
Оба WS_CLIPSIBLINGS, canvas явно HWND_TOP/NOACTIVATE и без TABSTOP. Provider
валидирует paint, сохраняет текст/caret/UIA и не рисует VT; native scroll
clipped. Его текстовые обновления не инвалидируют canvas. Mouse/canvas focus
перенаправляется в прежний EDIT; нижнего поля и второго места ввода нет.
Архитектура уточнена в ADR0010. Независимый узкий review не нашёл P1/P2.

**Проверки passed:** fmt, clippy(workspace/all-targets,-D warnings), **452tests**
(app389+worker3+PTY15unit/14integration+terminal31), release. Уменьшение счётчика
вызвано удалением тестов отменённой проекции меню. Логи
`target/paint-isolation/{fmt,clippy,test,build}.log`. EXE2228224bytes, SHA256
`E902C77173F5FC72F0B164A4791A57BC80BD2DC33CD5CF8C42022DA3ED3E5DAE`.
Копии `target/release/bks-app.exe` и `target/release/bks-app-0.4.0-preview.5.exe`
совпадают с `target/direct-accessibility/release/bks-app.exe`. Прежний default
сохранён в `target/release/bks-app.before-0.4.0-preview.5-14657d53bc944791a6b1d936ef811b92.exe`.

**Native isolation passed:**15passed/1skipped/0failed/error,
`target/paint-isolation/native/run-20261027-173109-6f9c9220/summary.json`.
Visible/samebounds/Z-order/clipping, readonly named TextPattern и
IsOffscreen=false подтверждены. Native-only replacement/scroll реально
применён (FirstVisibleLine130), VT неизменён:0 changed из663552 пикселей,
147 цветов. Реальный VT spinner positive-control:145 changed pixels.
Семь последовательных ticks сохранили readable/on-screen provider и overlay,
0PTY. Root просмотрел own PNG. Это доказательство изоляции, не человеческая
оценка мерцания. Keyless CliHandoff —14passed/0failed/error,
`target/paint-isolation/handoff/run-20261027-173121-f8ca11ff/summary.json`.
Собственные PID12000/6124 и10788/19120 закрыты штатно и отсутствуют;
FG/SetFocus/keyboard helpers не использовались. Набор/NVDA и отсутствие рывков
в настоящем Kimi после смены слоя ещё проверяет человек.

**Пакеты:** preview.5 portable/setup/manifest/SHA256SUMS в `target/packages`;
extraction/hash checks passed, private install/update/uninstall48passed/1skipped
(file-symlink Windows1314), без реальных registry/shortcuts/пользовательского
запуска. Логи `target/paint-isolation/package-{build,private}.log`.
Git whitespace check passed; commit/push/публикации не было.

**Открыт чистый пользовательский PowerShell preview.5:** appPID13016,
PowerShellPID15280, mainHWND2885274, exe`target/release/bks-app-0.4.0-preview.5.exe`,
caption`BKS Term — 0.4.0-preview.5`, cwdпроекта. Readonly metadata:
NativeCommand1002Visible=true, Journal1001Visible=true, Direct1011Visible=false.
Клавиши/команды не отправлялись, интерактивный Kimi не запускался; startup
version/help probes не являются пользовательской PTY. Старые пользовательские
окна14464/10232 не трогались. Metadata `target/paint-isolation/user-launch.json`.
Это окно для пользователя; не закрывать/не вводить автоматически и перепроверять
PID/HWND перед дальнейшими действиями.

**Отдельно выявлено, вне этой правки:** vte буферизует CSI2026 кадр, но его
150ms timeout сейчас не обслуживается на poll без новых байтов. При штатном
Kimi END_SYNC кадр атомарен; наличие sync в установленном Kimi подтверждено
read-only JS scan, прохождение через конкретный ConPTY не захватывалось.
Не добавлять stop_sync на каждую пачку и произвольный debounce: это разрушает
атомарность. Точечное обслуживание просроченного sync остаётся отдельной работой.

### Предыдущий результат — 27.10.2026, 0.4.0-preview.4

**Запрос пользователя:** набор в Kimi ещё молчал, выбор озвучивался, но
дёргался. Поручено нативное меню в зелёно-чёрном стиле также для слеш-команд.
Исправлен подтверждённый дефект echo: Kimi заполняет строку пробелами,
добавляет боковые границы и единственный inverse-курсор, скрывая hardware
cursor. Ввод расходует заполнение вместо роста строки. Новый путь сравнивает
точную вставку между курсорами и сохраняет окружающий текст, допускает
перемещение composer и Unicode. Подмена/маска отменяет pending до позднего
reveal; SGR8/невидимые ячейки не озвучиваются. Отдельно подтверждённое эхо
перенесено после native text/caret updates. Это исправление кода, не NVDA-pass.

**Общее меню:** `tui_menu.rs` консервативно выделяет соседние выровненные
choice/slash строки с единственным marker/inverse/background hint. Номера
версий CLI не требуются; неоднозначность сохраняет полный обзор. Явный
Ctrl+Shift+D открывает распознанный список как native LISTBOX/HASSTRINGS с
зелёно-чёрным owner-draw и системным High Contrast. Generic menu сразу начинает
literal управление; по тексту пунктов никакие действия не строятся. Сохраняются
source/input/fullframe проверки, one-key pending и защита от held Enter.
Tab/Shift+Tab обходят нативные элементы, в полном контексте стрелки и Enter
не передаются CLI. Ctrl+Shift+I сохраняет снимок. После явного Enter/Esc и
исчезновения списка возвращается ввод той же сессии. Для изменения фильтра
нужно вернуться к обычному вводу CLI; меню не открывается само от вывода.

**Стабильность:** выбор меняет существующий LISTBOX без LB_RESETCONTENT;
неизменные caption/accessibility names не переустанавливаются, контекст
сохраняет выделение/scroll. Строки рисуются через backbuffer. Raw EDIT
перехватывает WM_SETREDRAW без снятия WS_VISIBLE; native provider остаётся
доступным при пакетном обновлении. Обычный shell editor не менялся.

**Проверки passed:** fmt, clippy(workspace/all-targets,-D warnings), **463tests**
(app400+worker3+PTY15unit/14integration+terminal31), release. Логи
`target/tui-menu-echo/{fmt,clippy,test,build}.log`. Независимый один проход
ревью выявил mask→reveal в старом hidden-cursor fallback и перехват Tab/стрелок
контекста; оба P2 исправлены, целевой review подтвердил закрытие по коду и
регрессионным тестам. Echo26, menu9, native UI13 и две main integration
регрессии включены в общий прогон. Ни эти тесты, ни ревью не заменяют NVDA.

**Сборка:** 0.4.0-preview.4,2269184bytes,SHA256
`BDCBC762785775FF1A992A7DE4DEA2B65FBFD94B24B76BE17138D301DFDE271C`.
Обновлены `target/release/bks-app.exe` и
`target/release/bks-app-0.4.0-preview.4.exe`, исходник сборки
`target/direct-accessibility/release/bks-app.exe`. Прежний default сохранён в
`target/release/bks-app.before-0.4.0-preview.4-17dc4efd2ed9433494f1c303687b8bda.exe`.
Portable/setup/manifest/SHA256SUMS подготовлены в `target/packages`;
extraction/hash checks passed. Private install/update/uninstall **48passed/1skipped**
(file-symlink Windows1314), реальные product registry/shortcuts не трогались.
Логи `target/tui-menu-echo/package-{build,private}.log`.

**GUI частичная:** `-NativeMenu` остановлен до ввода —4passed/0failed/1error,
foreground refusal после одной штатной попытки активации. Raw пустой;
собственные PID5872/11680 закрыты без Kill. Отчёт
`target/native-menu/native/run-20261027-170823-f8ce1b80/summary.json`.
Native live navigation/Tab/echo по этому запуску НЕpassed. Ограничение FG
не ослаблялось, повторные активации не выполнялись. Последующий keyless
handoff — **14passed/0failed/error**,
`target/native-menu/handoff/run-20261027-171040-4caefd83/summary.json`.
Статический `-NativeMenuInspect` — **12passed/1skipped/0failed/error**,
`target/native-menu/inspect/run-20261027-171150-f01c2c86/summary.json`:
точные native/UIA choice/slash labels/descriptions, HASSTRINGS/ownerdraw,
полный readonly context, WM_SETREDRAW false/true сохраняет visible1011 и
provider text, итог0PTY. Root просмотрел `keyless-native-menu.png` и
`keyless-slash.png`: моноширинный список, зелёное выделение/тёмный градиент,
полный контекст ниже. Skip явно относится к live navigation/Tab/echo/NVDA;
фоновый BeginControl штатно замер без FG. Все собственные app/fixture PID
обоих запусков закрыты штатно и отсутствуют. Скрипт сохранён с отдельными
aligned native и staggered fallback fixture; PrepareOnly3passed/1skipped,
прежний default actual после этого не запускался. Человеческая NVDA-приёмка открыта.

**Пользовательские окна:** preview.3 PID10232 сохранён; его Kimi/клавиши/окно
не трогались. Открыт новый чистый PowerShell preview.4: appPID14464,
PowerShellPID19548, mainHWND5048462, exe`target/release/bks-app-0.4.0-preview.4.exe`,
nativecaption`BKS Term — 0.4.0-preview.4`, cwdпроекта. Readonly metadata:
NativeCommand1002Visible=true, Journal1001Visible=true, Direct1011Visible=false.
Клавиши/команды не отправлялись, интерактивный Kimi не запускался. Фоновые
version/help probes CLI при старте не являются пользовательскими сессиями.
Запуск записан в `target/tui-menu-echo/user-launch.json`. Новое окно для
пользовательской NVDA-проверки; не закрывать и не вводить автоматически,
PID/HWND перед действиями перепроверять. Commit/push/публикации не было.

### Предыдущий результат — 27.10.2026, 0.4.0-preview.3

**Продолжение после жалобы «оболочка ещё выполняет команду»:** metadata
подтвердила, что Kimi уже запущен под PowerShell пользовательского preview.2;
проблемой было оставшееся командное поле. Простой явно отправленный запуск
`kimi`, `codex`, `claude` или `gemini` теперь автоматически включает прямой
ввод в том же PTY. Аргументы, составные команды и неизвестные инструменты
оставляют ручное переключение. Перенаправления не распознаются как запуск CLI.

**Возврат и границы:** свежие OSC133 A/B должны содержать прежнее приглашение
родительской оболочки. Чужой prompt и одиночный B не открывают редактор;
последующее чужое приглашение отменяет прежнюю готовность. Захват prompt
ограничен4096 UTF16-единицами и обрабатывает текущий chunk вместо повторного
сканирования журнала. Reader/диалог сохраняются до явного возврата. Черновик
и старый Enter CLI не исполняются при переходе. Переключение режима не меняет
размер ConPTY. Одно независимое ревью выявило два P2 (перенаправления и
неотменяемая готовность); исправления проверены целевыми и общими тестами.
Повторный целевой ответ reviewer недоступен из-за backend HTTP403; закрытие
проверено root по коду и регрессионным тестам, без заявления нового review-pass.

**Большой пустой участок:** ранний resize ConPTY воспроизведён на собственных
PowerShell-процессах: начальная разметка содержала39 CRLF перед приглашением.
После начальной CSI2J эти строки не добавляются в журнал до первого печатного
символа. Raw VT не меняется; отступы и последующие пустые строки сохраняются.
Три реальные пробы дали39→0; доказательства в `target/cli-handoff-gap`.
Повторы старого экрана при последующем физическом resize этим не устранены.

**Rust-проверки passed:** fmt, clippy(all-targets,-D warnings), **442tests**
(app379+worker3+PTY15unit/14integration+terminal31), release. Логи
`target/cli-handoff-gap/{fmt,clippy,test,build}.log`. EXE2223104bytes,
SHA256 `CDDE052E3EC3700C99886F05C872D621D3E50A3124F3E89011B1AE5860EF40CB`.
Обновлены `target/release/bks-app.exe` и версия
`target/release/bks-app-0.4.0-preview.3.exe`; исходный EXE находится в
`target/direct-accessibility/release/bks-app.exe`. Прежний default сохранён
как `target/release/bks-app.before-0.4.0-preview.3-41e48019d6294d9b9974f16ce5f8adcc.exe`.

**GUI-проверка passed:** keyless `-CliHandoff` — **14passed/0failed/0skipped/0error**,
`target/cli-handoff/native/run-20261027-163723-262af8ea/summary.json`. Два цикла
native1002→direct1011→native1002, ровно два явных `kimi\r\n`, итог raw
`6B696D690D0A6B696D690D0A`. Один fixture PID9848, постоянный PTY113×32.
Во втором цикле reader сохранил selection467..489 и viewport0 после выхода CLI;
явный возврат60003 восстановил поле команды без дополнительных PTY-байтов.
Root просмотрел обе PNG: единая область CLI без нижнего поля, выделение
сохраняется при уже показанном приглашении оболочки. Это visual evidence,
не NVDA. Все6 собственных app/fixture PID трёх actual-запусков завершены
без Kill; `target/cli-handoff/owned-cleanup-check.json`. Первые два errors
сохранены: C# fixture ошибочно разбирал `\x07fake` как U+07FA вместо BEL+текста.
Исправлен только fixture на `\u0007fake`; строгий prompt oracle сохранён,
production EXE не менялся. Foreground/focus/keyboard helpers и настоящий CLI
не использовались. Новая NVDA-приёмка остаётся ручной.

**Пакеты preview.3:** portable/setup/manifest/SHA256SUMS в `target/packages`.
SFX/extraction/file hashes passed; после последней правки включённых документов
повторены упаковка и private install/update/uninstall: **48passed/1skipped**
(file-symlink Windows1314). Реальная product registry/shortcuts и пользовательский
запуск установщика не выполнялись. Логи
`target/cli-handoff-gap/package-{build,private}.log`. Diff whitespace check passed.

**Пользовательские окна:** preview.2 PID12692 был закрыт пользователем до
попытки переключить его режим. Guard остановил действие до WM_COMMAND;
переключение и клавиши не отправлялись. Старые PID/HWND исторические,
перед любыми действиями перепроверять. Самостоятельный запуск Kimi остаётся
за пользователем.

**Открыто чистое пользовательское окно preview.3:** TerminalPID10232,
PowerShellPID18676, mainHWND2162818, exe`target/release/bks-app-0.4.0-preview.3.exe`,
cwdпроекта, nativecaption`BKS Term — 0.4.0-preview.3`. Readonly-проверка:
NativeCommand1002Visible=true, Journal1001Visible=true, Direct1011Visible=false.
Команды/клавиши не отправлялись, интерактивный Kimi не запускался. Фоновый
discovery-probe CLI при старте не является пользовательской PTY-сессией.
Metadata запуска — `target/cli-handoff-gap/user-launch.json`. Это пользовательское
окно для ручной NVDA/Kimi-проверки; не закрывать и не вводить автоматически.
PID/HWND перед дальнейшими действиями перепроверять. Commit/push/публикации не было.

### Предыдущий результат — 27.10.2026, 0.4.0-preview.2

**Открыто новое пользовательское окно preview.2:** TerminalPID12692, PowerShellPID11500, mainHWND4849826, exe`target/release/bks-app-0.4.0-preview.2.exe`, cwdпроекта. После запуска readonly-проверка подтвердила NativeCommand1002Visible=true, Journal1001Visible=true, Direct1011Visible=false. Nativecaption задан`BKS Term — 0.4.0-preview.2` для различения Alt+Tab/NVDA. Только чистый PowerShell, без интерактивного Kimi и без отправки клавиш/команд. Старые пользовательские10948/19496сохранены. Это НЕQAокно: не закрывать/не вводить автоматически. PID/HWND перед дальнейшими действиями перепроверять.

**Ручная регрессия preview.1 признана и исправлена:** пользователь на чистом PowerShell сообщил о лишней белой мигающей каретке ниже VT-курсора, неработающих Ctrl+Up/Down/Tab, выделении и устаревшем тексте после удаления. Причина — ошибочно включённый direct/capture для ВСЕХ профилей и raw Ctrl+Up/Down passthrough. Это не недостаток настроек NVDA пользователя. Обычные оболочки теперь снова используют прежние nativeEDIT1002/журнал1001, выделение/удаление и GUI Tab. Только явные LocalCLI-профили cli-* и legacycodex стартуютdirect; ручнойTUIдоступенчерезCtrl+Shift+I. Ctrl+Up/Down в shell используютблокижурнала(безблоковnativefallback), в TUI переходяткчтениюсоседнихлогическихстрок0PTY.

**Лишняя каретка:** system_caret::hide_visible проверяет GetGUIThreadInfo текущегоUIthread, точныйhwndCaret иGUI_CARETBLINKING; HideCaret вызывается только для видимой собственной каретки послеFocus/Size/Font/EMupdates/Paint и при50ms poll. Объект/позиция не уничтожаются, счётчик не накапливается. NativeWin32 test проверяетидемпотентность/одинShowCaret/пересоздание. Main/caret/profileдельта прошлацелевойreview, открытыхP1/P2 не найдено.

**Итоговые проверки preview.2 passed:** fmt, clippy, **434tests** (app371+workerprocess3+PTY15unit/14integration+terminal31), releasebuild; логиtarget/caret-navigation/{fmt,clippy,test,build}.log. EXE2212352байта, SHA256`2BD1F4441BCFBDC6145DF0B7F928CE80E67E9C19826210396313A6A134F15018`. Пользовательскаякопия`target/release/bks-app-0.4.0-preview.2.exe`; исходнаяtarget/direct-accessibility/release/bks-app.exe. Пакетыpreview.2вtarget/packages, SFX/extraction/filehashespassed; privateinstall/update/uninstall48passed/1skipped(file-symlink1314), логtarget/caret-navigation/package-private.log.

**GUI приёмка preview.2 частичная:** первыйShellRegression подтвердилnativelayout/Tab/Unicodeввод/ShiftRightselection/UIA/Backspace/стрелки0PTY иотсутствиеstale custom notifications (8passed), затемоснасткаожидалаCRвместоштатногоCRLF; исправленаТОЛЬКОоснастка. ПоследующиеShell/ReadNavigationзапускиостановленыдо клавиш из-заforegroundrefusal. CtrlUpDownпоблокам, readerEM_GETSEL иrawhidden-caretlifecycle НЕpassed; не подменять ими прошлый35passedpreview.1. НоваяNVDAпроверкаручная. Скриптcheck-unified-tui.ps1имеет-ShellRegression/-ReadNavigation, PrepareOnly3passed/1skipped. ВосемьсобственныхQA PIDзавершены; чужие/пользовательские10948/19496неактивировалисьине закрывались. Артефактыtarget/caret-navigation/{shell,read,owned-cleanup-check.json}.

### Предыдущий результат — 27.10.2026, 0.4.0-preview.1

**Новый пользовательский запуск0.4:** по прямому запросу «запускай новый терминал, чистый, без кими, я буду сам всё запускать» открыто видимое окно `target/release/bks-app-0.4.0-preview.1.exe --profile windows-powershell --cwd <кореньпроекта>`. TerminalPID19496, PowerShellPID12020, mainHWND5310670. Профиль PowerShell использует-NoProfile; Kimi интерактивно не запускался, пользователь запускает инструменты сам. Новое окно — пользовательское для ручной NVDA-проверки, не QA: не закрывать и не отправлять клавиши/команды автоматически. Старое окно10948 не закрывалось. Перед дальнейшими действиями PID/HWND перепроверять.

**Поручение после ручного Kimi-теста:** пользователь разрешил реализовать все пять согласованных пунктов последовательно, без промежуточных подтверждений. Реализованы единая рабочая область, видимый набор/чтение строки, переход к новому выводу, нативный диалог меню с явным live-управлением и общий fallback для разных TUI. Commit/push/публикация не разрешены.

**Реализация:** native read-only EDIT1011 занимает область самого терминала и рисует прежнюю GDI VT-сетку через subclass; нижнего дублирующего поля нет. Новые профили сразу direct/captured; EN_SETFOCUS восстанавливает управление. Ctrl+Shift+Space/Esc из reader возвращают ввод и Tab. Ctrl+Shift+L читает текущую логическую строку (до4096скаляров), Ctrl+Shift+R выбирает начало изменений после Enter и читает первую строку. InputEcho подтверждает точные видимые вставки Unicode, ограничен64скалярами/750мс; navigation/focus отменяют старый anchor, HIDDEN отменяет echo до продления pending и на poll. Нет speculative вставки/снятияREADONLY. ReplyAnchor хранит4096отпечатков видимых строк. Native TUI dialog4405 явно включает literal keys, pending/защиту от repeat/stale; snapshot остаётся стабильным по умолчанию. Mapped Codex остаётся отдельным механизмом. Детали: ADR0010.

**Conceal-регрессия исправлена:** SGR8 текст скрывается в CellSnapshot/ReadingDocument/журнале, UTF16диапазоны и gridwidth сохранены. Normalizer учитывает colors operands, saved attrs ESC7/8/CSI s/u,1049 иRIS. Не используются эвристики паролей. После общего review исправлены stale hidden-cursor anchor и typed-обходconcealguard; целевые замечания закрыты, открытыхP1/P2 в проверенномdelta нет.

**Итоговые Rust-проверки passed:** fmt, clippy(alltargets,-Dwarnings), **431tests** (app368+workerprocess3+PTY15unit/14integration+terminal31), releasebuild. Логи target/unified-tui/{fmt,clippy,test,build}.log. ИтоговыйEXE2207744байта, SHA256`689127B336411EAFF0DA5BB480A40E7BDA09524494B6A477C5713F2B2388FBD1`. Отдельная копия для пользователя: `target/release/bks-app-0.4.0-preview.1.exe`; исходная сборка `target/direct-accessibility/release/bks-app.exe`. Пакеты0.4 подготовлены вtarget/packages; SFX/extraction/filehashes passed. Private install/update/uninstall: **48passed/1skipped** (file-symlink Windows1314), без реальной интеграции/запуска пользователя. Логpackage-private-winps.log: выполнялся WinPS5 с его штатным Modulespath; первая попытка изPowerShell7 провалилась из-за inheritedPSModulePath/Get-FileHash, production installer не менялся.

**Native unified GUI passed:** **35passed/0failed/1skipped/0error**, `target/unified-tui/native/run-20261027-141355-86f94375/summary.json`. Проверены default/fullarea/TextPattern, реальное повторное получение фокуса/Tab, Ctrl+Up/Down raw, независимое чтение/Esc, snapshot0bytes, liveUp/Down/Enter/Tab/Esc ровноодинраз, heldEnter, pending~700мс, staleEnter0bytes, отсутствие speculativeecho, Кириллица/emoji→PTY, подтверждённая UTF16каретка/UIA, Ctrl+Shift+L0bytes и Ctrl+Shift+Rfirstchangedline/Esc0bytes/Tab. Skip: native adapter-fault injection, покрывается существующими Rustworkertransporttests. Own PNGdirect-surface/live-menu визуально просмотрены root: единая область безнижнегодубля и доступный моноширинный диалог. Private app/fixture закрыты штатно. Начальные сбои оснастки сохранены: foreignforegroundдоактивации, PrintWindowchildrefusal, затем ReadFile/ReadConsoleA заменял emoji на2U+FFFD. Исправлен ТОЛЬКОfixture collector на ReadConsoleW+statefulUTF8, подтвердивший IsWindowUnicode и raw numericD83DDE00→F09F9880 на неизменномEXE; это не обход production-багa.

**Существующая пользовательская сессия сохранена:** PID10948 по-прежнему запущен изtarget/release/bks-app.exe (0.3.0-preview.1); его executable не заменялся, команды/клавиши в Kimi не отправлялись. Новая NVDA-приёмка не выполнялась агентом и не считаетсяpassed. Native QA использует только собственные PID/HWND/privateAPPDATA и однократную активацию собственного окна; ввод/фокус на чужие окна не направлялись. Для следующей ручной проверки запускать новую versioned-копию EXE, а Kimi пользователь запускает самостоятельно.

### Предыдущий результат — 27.10.2026, 0.3.0-preview.1

**Ручная проверка Kimi, 27.10.2026, обратная связь пользователя:** Kimi2.3.3 запущен пользователем из PowerShell. Начальный вывод читается, декоративная псевдографика игнорируется, меню команд читается. При наборе текста озвучивания нет; пользователь не может удобно перечитать свой запрос и ответ Kimi. Скриншот115841 показывает «привет» в TUI и старую строку PowerShell в нижнем нативном поле. Это отрицательный результат соответствующей ручной приёмки, а не завершённая доступность ввода/ответов. По коду прямой ввод1011 —read-only EDIT, клавиши перехватываются до штатного редактирования; NVDA по умолчанию ограничивает echo редактируемыми полями. Одного снятияREADONLY недостаточно для корректного общего решения: прямой EDIT не знает, когда TUI скрывает пароль. Пользователь предлагает диалог в стилистике терминала и ставит под вопрос отдельный переход к полю ввода. Обсуждается единая рабочая область с доступным вводом/перечитыванием запроса, удобным чтением вывода и нативными меню. В этом ходе production-код, EXE и пользовательские процессы не изменены; общая схема нового интерфейса ещё не реализована. Прежний итог автоматических проверок377tests остаётся историей предыдущей сборки и не подтверждает эти NVDA-сценарии.

**Пользовательская ручная сессия:** пользователь уточнил «хочу с нуля, сам запустить… я про кими». Автозапущенный интерактивный KimiPID8040 в единственном нашем окне10260 остановлен, старое окно закрыто штатно. Открыта чистая Windows PowerShell-сессия в новом BKS Term0.3.0-preview.1: TerminalPID10948, PowerShellPID13796, mainHWND985200, cwdпроекта. Kimi пользователь запустит САМ; не вводить команды и не запускать CLI автоматически. Это пользовательское окно для NVDA, не QA: не закрывать. PID/HWND перед дальнейшей работой перепроверить. Фактический результат NVDA ещё не получен. Ранее записанный18956 был краткоживущим version probe, а не интерактивной сессией.

**Разработка и упаковка закончены в доступном автоматическом объёме; ручная приёмка открыта.** Последний managed-CLI дефект исправлен: короткий обычный canonical DOS/UNC путь используется после проверки компонентов и тождества повторной canonicalize; исходное launch evidence сохранено. Реальный regression проверяет Session::spawn → .NET Configuration/JavaScriptSerializer → Unicodecwd/marker/exit0. Чужие foreground/процессы не трогались. Native normal new-window/Keep/Close прошли14checks; последний Runtime cold startup дал9passed/0failed/1error только при попытке открыть chooser из-за foregroundRDP (run20261027-113556-673a3883). Сам cold runtime peer теперь запущен корректно; combo new-window CLI ещё не passed.

**Последний код и пакеты:** оба startup-дефекта исправлены: сохранённое launch FileStamp evidence связывает позднюю версию с реально выбранным executable, а deferred explicit runtime profile сохраняетcwd и отменяется другим выбором сессии. Никакого fsIO/version probe на UI timer. Полные fmt/clippy/tests/release —passed, **377tests** (app318+workerprocess3+PTY15unit/14integration+terminal27), target/tui-completion/release-final-*.log. Один общий проход review и целевые проверки startup/path-исправлений не нашлиP1/P2. EXE и пакеты пересобраны после всех production Rust-правок; прежниеcheckpoint ниже — история. Все собственные тестовые процессы закрыты, пользовательские окна не закрывались. Новое пользовательское окно не запускалось поверхRDP.

Дополнительный native run target/tui-completion/final-direct/run-20261027-104106-072e5f1b/summary.json дал55passed: readonly overview, stale context/selection preservation, return UIA Invoke, bg/inverse-only feedback успешно. Последний timeout 'field input mode restored' оказался старой ошибкой оснастки: MENU60001 не WM_COMMAND; заменён на реальный button1009. Следующийrun104319-73e0331b дал48passed/1error из-за ухода foreground вChrome при ожидании feedback; source не обвинять. Метод.Current.GetSelection также уже исправлен. Rust/coreGUIэффекты после Announcerfix реально подтверждены, но единого полностью passed native run ещё нет.

Выполнена доступная автоматическая часть согласованных восьми пунктов. **Полная приёмка ещё не закрыта:** NVDA/зрительное мерцание, полный разговор и разрешения четырёх CLI, оставшиеся GUI-сценарии и чистая Windows/VM требуют фактической проверки. Пользователь разрешил продолжать без промежуточных подтверждений и делегировать; commit/push/публикация не разрешены.

1. Исправлены baseline оснастки и реальный второй foreground-gate в announcements.rs: оба HWND сравниваются по GA_ROOT. На итоговом EXE в partial native run прошли40проверок: живые UIA Notification после focus на childEDIT, Unicode/caret/точный ввод, reader/draft/F1/capture,100repaint без дополнительного ввода/feedback. Старые Settings preview.2 —19passed.
2. Общий feedback поддерживает reciprocal fg/bg/inverse и переход видимого курсора между строками; сохраняет90/800мс и ограничения. Тема, одиночные/неоднозначные изменения и пассивная перерисовка не становятся выбором.
3. Ctrl+Shift+D открывает snapshot любого TUI: native EDIT+LISTBOX строк, подсказка возможного выделения, чтение без PTY bytes и явный возврат к реальным клавишам. Код/unit готовы; новые полные native overview/style scenes ещё не прошли до конца (см. GUI ниже).
4. Изолированный self-worker, bounded JSONL/очереди, privateJob,2с startup/500мс response, async scan/cache, sealed source/context/selection и фиксированный mapper. Ошибка отключает улучшение, явный Ctrl+D разрешает retry; PTY не перезапускается. Две реальные независимые PTY пережили kill worker в integration test. ADR0009.
5. Фоновые профили Codex/Kimi/Claude/Gemini, optional AX Claude/Gemini после version/help. Неизвестная версия сохраняет normal profile. Native evidence Codex проверяется при запуске, не на timer. Probes имеют privateJob и isolated homes; тест подтвердил cleanup parent+descendant после10с timeout.
6. **Итоговые fmt/clippy(all-targets,-D warnings)/cargo test/release — passed,377tests** (app318+workerprocess3+PTY15unit/14integration+terminal27). Логи target/tui-completion/release-final-{fmt,clippy,test,build}.log. Независимое ревью и целевые проверки startup/path-фиксов не нашли P1/P2; повторный полный проход не нужен без новых существенных изменений. Первое падение нового lifecycle test было ошибкой stdout forwarding фикстуры, исправлено без изменения production/таймаутов; прежний лог сохранён.
7. Реальный headless startup четырёх CLI: **21passed/6skipped/0failed/error**, target/tui-completion/startup-probe/run-20261027-103248-bbea6312/assessment.json. Kimi2.3.3 Trust→Exit и Claude2.1.240 --bare Auto↔Dark дали правильный общий feedback; Escape Kimi вышел0. Codex0.164.0 и Gemini0.65.0 остановлены у auth boundary без ввода. Все4 privateJobs пусты, собственные процессы закрыты. Это не полные пользовательские workflows и не NVDA. Gemini установлен только под target/tui-completion/gemini-install, не глобально.
8. **Итоговый EXE:** target/release/bks-app.exe и target/direct-accessibility/release/bks-app.exe,0.3.0-preview.1,2130944bytes,SHA256 `CC3DFC610E91B383330E6D9CA21F5A598A3042C7FA0DEBC24F992564093AB19B`. Прежняяpreview.2 сохранена как target/release/bks-app.before-0.3.0-c08e51a99a3a48d39de1888e3051626e.exe; последняя промежуточная0.3 — target/release/bks-app.before-managed-fix-e857fb1ad2324c2ab83bf58863bbb6d5.exe. Portable1075116bytes,hash `d3d5d6d207bd224f695bfff14f51b183d17134f45efca2f2071222ade170b3c4`; Setup1103872bytes,hash `29aee634d7e28c9da80c49b6ec5b44ca4ba45f567f00c54898f936a4569ee6b1`. Оба в target/packages/bks-term-0.3.0-preview.1-windows-x64-*. Упаковка/extract/hashes passed; **48 private install/update/uninstall passed,1file-symlink skipped Windows1314**. Логи target/tui-completion/package-{build,private}-final.log; manifest/SHA256SUMS рядом. EXE/Setup unsigned. Реальные HKCU/StartMenu не изменены.

**GUI и следующий шаг:** единого полного passed-прогона ещё нет. Наиболее полный direct run104106-072e5f1b —55passed; два исправления оснастки после него: managed SelectionPattern.Current.GetSelection и button1009 вместо popup-only60001. Последующие direct прерывались уходом фокуса вChrome/RDP; production не объявлен defective по этим environment errors. Stage89 принимает GA_ROOT(foreground)==Main и сохраняет bounded failure diagnostics. Stage1011 имеет -Mode Runtime, cold/newwindow/zeroarg markers и cleanup GUI hosts перед tracked windowless workers; число GUI apps считается по main HWND, не по workers. Последний Runtime113556-673a3883 —9passed/0failed/1error externalRDP до chooser; startup/Unicodecwd/однаzeroargCLIsession и cleanup прошли. Старые normal new-window/Keep/Close прошли14checks в111445-8bbcfd95. Перед новым GUI прогоном нужен свободный интерактивный desktop и предупреждение о фокусе; не форсировать foreground и не повторять бесконечно. Последовательно выполнить direct, stage89 -Mode All, stage1011 -Mode All на актуальном target/release. Полный Cargo не повторять ради PowerShell/Memory без новых Rust-правок. PrepareOnly: direct3passed/1skipped, stage1011 5passed/1skipped, stage89 1passed/1skipped — не считать GUI passed.

WSL имеет только остановленный docker-desktop, daemonDocker не отвечает, SSH-хост и чистая VM не предоставлены. На async NVDA-вопрос пользователь ответил «Пока не проверял»; человеческая приёмка открыта. Новые пользовательские окна не запускались поверх RDP. MEMORY/setup не входят в пакет, их запись не требует повторной сборки/упаковки. Документация/F1/CHANGELOG обновлены, source docs в пакете совпадают с текущей версией. Commit/push/публикации не было.

### Ход реализации и исторические checkpoints

### Продолжение — 27.10.2026

Пользователь утвердил восемь пунктов завершения основной разработки и поручил выполнить их последовательно без промежуточных подтверждений; разрешил целесообразное делегирование. Пауза 26.10 снята. Порядок: (1) закрыть preview.2 harness/Settings; (2) общий TUI feedback, включая подсветку; (3) доступное чтение и навигация по типовым меню; (4) изоляция и отказоустойчивость адаптеров; (5) Codex/Kimi/Gemini/Claude; (6) совместимость и регрессии; (7) доступные автоматические проверки и отдельная человеческая NVDA-приёмка; (8) EXE/пакеты/документация. Основа — разные TUI; адаптеры необязательны, несовместимость не останавливает обычную PTY-сессию. Подробности — раздел «Завершение основной разработки» в docs/setup-and-iterations.md.

Разрешены локальные изменения, сборки, собственные тестовые процессы и необходимые GUI-прогоны с предупреждением о фокусе. Пользовательские окна сохранять. Реальные модельные запросы, чужие SSH-системы, commit/push/публикация не входят в автоматические проверки. NVDA, зрительное отсутствие мерцания и чистая Windows/VM не объявлять пройденными без фактической проверки.

**Промежуточный результат 27.10:** Settings на прежней preview.2 — 19 passed (target/tui-completion/stage1-settings/run-20261027-094306-13516ca5/summary.json). Direct run остановлен до синтетических клавиш: внешняя UTF-8 буква «б» D0B1; исходный отчёт 11 passed/1 failed сохранён, причина доказана как environment input, повторный guard оснастки исправлен на error. Baseline capture теперь коррелирован по точному Notification, пассивные focus/raw/feedback checks разделены. Повтор GUI отложен до итоговой сборки.

**Итерация общего TUI (пункты 2–3):** reciprocal перенос fg/bg/inverse и переход видимого курсора на другую строку дают один bounded feedback после явной навигации; массовые/одиночные/неоднозначные style changes тихие. Ctrl+Shift+D открывает read-only обзор любого CLI с контекстом и native LISTBOX строк; стрелки/Enter не отправляют ввод, отдельная кнопка возвращает в direct input. Позиция чтения сохраняется при изменении экрана. Проверенное evidence Codex сохраняется за запуском сессии, файловые проверки удалены из UI polling.

На этом checkpoint fmt/clippy(all targets,-D warnings)/cargo test/release — **passed**, 333 tests (app277+PTY15/14+terminal27). Логи target/tui-completion/stage23-{fmt,clippy,test,build}.log. EXE **0.2.0-preview.3**: target/tui-completion/stage23/bks-app.exe (1901056bytes). target/release/bks-app.exe пока остаётся прежней preview.2; target/direct-accessibility/release/bks-app.exe теперь preview.3. Извлечение OwnedChild для изоляции подготовлено, сами worker/host ещё не подключены. Следующий пункт: подключить worker-процесс адаптера и проверить отказоустойчивость; затем CLI-профили, общий regression, packaging. Человеческая NVDA-приёмка не выполнена.

**Пункт 4, checkpoint:** подключён isolated worker и sealed cache; единственный production caller recognize — tui_worker. Lazy scan после90мс, async Ctrl+D fallback максимум2.75с, background не открывает окно; failure disable и явный retry. Host имеет bounded queues/JSONL,2с startup/500мс response, privateJob и nonblockingDrop. Проверки fmt/clippy/tests/release — **passed**,357tests(app298+workerprocess3+PTY15/14+terminal27), логи target/tui-completion/stage4-*.log, EXE target/tui-completion/stage4/bks-app.exe. Версия переведена на **0.3.0-preview.1** согласно release policy для изменения озвучивания/новой возможности; старые checkpoint сохраняются.

**Новый подтверждённый дефект, ещё в исправлении:** baselineOnly на старой preview.2 (target/tui-completion/stage1-baseline/run-20261027-101253-09d35469/summary.json) дал14passed/0failed/1error: raw1B5B42 и caretBravo верны, foreground действительно childEDIT, но Notifications=[] и control1026 пустой. В announcements.rs обнаружен второй строгий gate root!=foreground, помимо уже исправленного main helper. Старый26.10 run имел foreground MAIN на момент Down, поэтому прежний вывод памяти о полном childforeground-path был неточным. Gate провайдера нормализован к GA_ROOT с обеих сторон; итоговые Cargo/native ещё предстоят. Collector не признан неисправным без доказательств.

### История: пауза по просьбе пользователя — 26.10.2026, вечер

**Последнее поручение:** «стопорнемся пока, в памяти проекта зафиксируй, на чем остановились, чтобы завтра продолжить с этого же места». Работа остановлена. До явного продолжения не править код/скрипты, не запускать проверки и новые окна. Это отменяет прежнее разрешение работать без остановок на время паузы. Сохраняем память и останавливаем собственные тестовые процессы; пользовательские окна/сессии не закрывать. Новую пользовательскую preview.2 ещё НЕ запускали: обещанный запуск отменён паузой. Все QA-приложения закрыты; агенты остановлены.

- **Причина текущего исправления:** пользователь отклонил прямой ввод preview.1 на реальном Kimi: фокус уходил в графический контрол без текста для NVDA, наблюдалось постоянное мерцание. Прежняя инструкция просто нажать Ctrl+Shift+I доступность не обеспечивала. Старые native/UIA passed не являются NVDA-приёмкой.
- **Готовый код preview.2:** новый native read-only EDIT1011 «Терминал — прямой ввод» под графической сеткой, живой VT-документ и UTF16-каретка; клавиши идут в ConPTY до стандартного EDIT. Полный reader1018 и его закладки независимы, draft1002 сохраняется. При входе direct источник чтения становится VT; Ctrl+Shift+Space открывает полный документ. `direct_accessibility.rs`: caret mapping history/wrap/wide/combining и один bounded ответ после явной навигации (90мс stable, 800мс lifetime, 300скаляров, до256строк/1024скаляров сравнения), без автоматических решений. Hidden cursor→EOF, изменения только цвета не озвучиваются; это общий текстовый доступ, не семантический адаптер Kimi.
- **Другие исправления main.rs:** enable capture вычисляется ДО save_active_view, поэтому повторное Ctrl+Shift+I действительно выключает захват. EN_SETFOCUS/EN_KILLFOCUS учитывают1011, потеря фокуса снимает захват. Раздельные terminal_rect/output_rect сохраняют размер canvas/ConPTY при полном чтении. `window_is_foreground` принимает main или GA_ROOT(foreground)==main: штатный UIA.SetFocus иногда делает foreground самим EDIT; строгая проверка main ранее ошибочно глушила уведомления. Чужие root/отдельные owned dialogs по-прежнему исключены.
- **Мерцание:** terminal_view.rs рисует кадр в RAII memory DC/bitmap и выводит одним BitBlt; GDI objects освобождаются. Добавлен настоящий memory-DC regression test без UI. Native journal инвалидируется без отдельного erase. Визуальное отсутствие мерцания на реальном Kimi ещё НЕ проверено человеком.
- **Окончательные обязательные проверки ПОСЛЕ всех Rust-правок — passed:** fmt, clippy workspace/all-targets `-D warnings`, **315 workspace tests** (app259 + PTY15unit/14integration + terminal27), release. Логи `target/direct-accessibility-final-fmt.log`, `-clippy.log`, `-test.log`, `-build.log`. Сборка: `cargo build --release --target-dir target/direct-accessibility`. Один независимый code-review и точечные проверки terminal_rect/foreground helper не нашли P1/P2. Нового полного ревью не нужно ради harness baseline.
- **Последняя запускаемая сборка:** `target/release/bks-app.exe` И `target/direct-accessibility/release/bks-app.exe` совпадают: ProductVersion **0.2.0-preview.2**, 1877504bytes, SHA256 **C5385DB8D77224FD5C689391D1700CE934FC5621EB4A103517F49C86EF594BD3**. Предыдущий пользовательский EXE сохранён как `target/release/bks-app.before-preview2-3df1cbf8934441348a1f915a8ddf97f0.exe`, промежуточный — `target/release/bks-app.before-foreground-fix-e729d6d4050a4d29b480247112e94528.exe`. Старую сессию17456/Kimi не закрывали и клавиши туда не отправляли; наличие этого PID завтра не предполагать без проверки.
- **Последний native/UIA прогон НЕ полностью passed:** `scripts/check-direct-accessibility.ps1` с `scripts/fixtures/direct-accessibility.cs`, отчёт `target/direct-accessibility-check/run-20261026-233023-9a21e50c/summary.json`: **35 passed / 1 failed / 0 skipped/error**. Own app11596 и fixture закрыты штатно, cleanup passed. Подтверждены TextPattern/точное имя/Unicode caret, реальные Notification после UIA.SetFocus с foreground child, ↑↓/Enter/Esc/ASCII/Ж ровноодинраз (`1B5B421B5B410D1B78D096`), reader/bookmark/draft, неизменные canvas+PTY size, F1, два capture on/off, отзывчивость при100repaint. **Остался combined fail** `passive repaint preserves focus and sends no input or extra direct feedback`: фокус1011/raw верны; лишним событием было запоздавшее ЯВНОЕ «Прямой ввод…» от MENU60004 перед repaint, baseline счётчика взят до асинхронной доставки. Пассивные Tick/row события в журнале не найдены; продуктовый дефект пока не доказан. Baseline ещё НЕ исправлен, после этого run изменений harness не сохраняли.
- **Важные уже исправленные ошибки тестовой оснастки, не возвращать:** native multiline EDIT предоставляет TextPattern, отсутствие ValuePattern нормально (Microsoft Learn: `https://learn.microsoft.com/en-us/dotnet/framework/ui-automation/implementing-the-ui-automation-value-control-pattern`). Сравнение nativeBefore/UIA/nativeAfter требует стабильного nativeBefore==nativeAfter. Для live marker используется LastIndexOf, потому что прежние кадры могут остаться в ConPTY history. KEYDOWN LPARAM1, KEYUP LPARAM0xC0000001, WM_CHAR LPARAM1; старый KEYUP LPARAM0 через TranslateMessage порождал дополнительный WM_CHAR13 и ложный0D0D. Невидимый message-only regression probe это доказал и включён в PrepareOnly (3passed/1skipped). Не «чинить» production Enter по этому старому ложному падению. Одна ранняя GUI-попытка загрязнена внешней буквой «в»; при неожиданных bytes тест фиксирует environment error, не product pass. UIA.SetFocus child остаётся настоящим тестом, SetForegroundWindow обход не применять.
- **Упаковка preview.2 уже готова:** `target/packages/bks-term-0.2.0-preview.2-windows-x64-portable.zip` 967309bytes, SHA256 `88f7b4710040b1c63027cc234b715791b9c8e3848418bd3ff8899075eeb86897`; Setup995840bytes, SHA256 `637f2e1d3f9aaee5fb31bf7b41ef1ec7cdd58d0db40d5ea5cdf275086fe4dd38`. Отчёт `target/package-verification-final-preview2-20261026-232941/summary.json` существует: packaging/hashes/16sources passed, **48 private install/uninstall passed, 1 skipped**, 0failed/error. File-symlink skipped Windows1314, пять junction/sentinel passed. Источники не менялись, preview.1 пакеты сохранены; оба EXE unsigned/asInvoker, реальный HKCU/Start Menu не затрагивались. Первый hidden запуск получил error из-за наследования PS7 PSModulePath; повтор только с системным WinPS5.1 module path дочернего процесса passed. Логи внутри каталога отчёта: `package-build-final.log`, `package-private-smoke.log`. Агент release_tools прерван по STOP при финальной сверке; новые запуски не нужны для уже готового отчёта.
- **Документация обновлена:** README, F1/help.rs, docs/help.md, manual-final.md (добавлены5шагов direct regression), known-limitations.md, CHANGELOG, ADR0004, setup-and-iterations.md. `scripts/check-stage1011.ps1` обновлён под фокус1011 вместо1017 и одновременную видимость canvas; этот изменённый сценарий Settings ещё не запускали. MEMORY не входит в пакет: её запись не требует пересборки/переупаковки. Речь NVDA/реальный Kimi/flicker/чистая VM остаются manual, CI cloud не запускался. Commit/push/публикации не разрешены.

**С чего начать завтра после явного продолжения:** исправить ТОЛЬКО baseline в последнем native harness: дождаться доставки явного capture Notification перед счётчиком, разделить проверки сохранения фокуса, отсутствия PTY bytes и отсутствия пассивного feedback; добавить before/after observations. Затем один финальный run на существующем release и подходящий Settings-сценарий (GUI проверки последовательно). Не повторять Cargo ради правки одного PowerShell harness: код/EXE уже проверены. Если подтверждён новый Rust-дефект — исправление и обязательные проверки/новая упаковка по изменённым входам. Тестовое окно получает фокус: перед GUI предупредить пользователя, при внешнем вводе остановиться, не делать бесконечных повторов. После проверок открыть новую preview.2 (желательно отличить заголовком версии, старую пользовательскую сессию не закрывать), затем продолжить ручной NVDA/Kimi тест ПО ОДНОМУ шагу. До этого не утверждать «всё доступно» или «мерцание устранено».

### История сдачи0.2.0-preview.1

- **0.2.0-preview.1 готова для ручной приёмки.** По последнему поручению пользователя завершены этапы8–12, исправления, встроенная справка F1, руководство, CHANGELOG, инструкция установки/удаления, ограничения и финальные ручные сценарии. Дополнительных подтверждений пользователь не требует. Commit/push, публикация, реальные запросы модели и подключения к чужим системам не выполнялись и не разрешены этим поручением.
- **Обязательные проверки — passed:** `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (**299**: app243 + PTY15unit/14integration + terminal27), `cargo build --release`. В том числе 32 последовательных реальных жизненных цикла ConPTY с нормальным выходом и явным завершением. Лог: `target/stage1011-tests.log`. Последующие изменения документации не меняют исполняемый файл и не требуют повторного прогона Rust.
- **Финальная native/UIA QA — 86 passed, 0 failed/skipped/error:** 50 проверок8–9 (`target/stage89-check/all-20261026-194648-e2d27bcc/summary.json`) и 36 проверок10–11 (`target/stage1011-check/run-20261026-194011-9f8b9668/summary.json`). Использовались только собственные PID/HWND, отдельный APPDATA и локальные фикстуры, все собственные тестовые процессы закрыты. Это не проверка речи NVDA.
- **Запускаемый EXE:** `target/release/bks-app.exe`, 1852416bytes, ProductVersion0.2.0-preview.1, SHA256 `7FFE38B0E27A139A045F976ED26E0A8DD7E692F1489AFE8B567A4DE22F8AE510`. Копии `target/stage10/bks-app.exe` и `target/stage11/bks-app.exe` совпадают. Старые stage8/9 checkpoints ниже — история, тестировать следует окончательную версию.
- **Возможности8–9:** Important/All/Manual с подавлением redraw/фонового повтора; доступный нативный TUI-диалог только для Codex CLI0.164.0 startup Folder access отдельного проверенного профиля, полный контекст и явное применение. Актуальность session/VT/input revision и очередей проверяется атомарно; чтение/закрытие не отправляет клавиши.
- **Возможности10–11:** opt-in Codex App Server0.164.0 text frontend с stream/Interrupt/разовыми обычными command approvals и Job cleanup; реальные модель/auth-запросы не делались. Новые окна/папки, настройка клавиш с неизменяемыми recovery keys, последняя вкладка, opt-in MRU, явные Explorer verbs, config lock/latest merge/CAS, доступная через Tab SSH-подсказка. Обычный старт не запускает сохранённый SSH/default command. ADR0007/0008 фиксируют границы.
- **Этап12 завершён, финальные документы включены:** portable ZIP и настоящий per-user Setup EXE в `target/packages`, статический MSVC CRT, версия/manifest/SHA256/DLL report, безопасные upgrade/uninstall с сохранением настроек. Неподписанные asInvoker сборки. Итоговая упаковка: **48 passed, 1 skipped, 0 failed/error**, отчёт `target/package-verification-final-20261026-200435/summary.json`; частная установка без product HKCU/Start Menu. File-symlink test skipped из-за Windows1314; пять реальных directory-junction/sentinel проверок passed. ZIP `bks-term-0.2.0-preview.1-windows-x64-portable.zip`: 954471bytes, SHA256 `091a87ea7cac2f4fee148486c90db9c7f67b5ef60e749fb927f67820b7d961f8`. Setup `bks-term-0.2.0-preview.1-windows-x64-setup.exe`: 983040bytes, SHA256 `47709c6ae285ce851fa5b267da92bb5cca84c59d48effd36210a467de1964e40`. Логи `target/package-build.log`, `target/package-final-check.log`. Workflow Windows подготовлен и actions закреплены; облачный CI без push не запускался.
- **Остаётся ручная приёмка, а не новый код по этому плану:** `docs/manual-final.md` (NVDA, реальный CLI/App Server/SSH/WSL/Docker, системные DPI/High Contrast, Explorer и чистая VM). Автозахват живого Codex был отклонён автоматической проверкой `blocked by policy`; настоящий trust screen не подтверждён. Supplementary input Git Bash теряет пару после native ConPTY на пути Git/MSYS; безопасный обход не найден, граница доказана и описана в `docs/known-limitations.md`. Ничего из этого не выдавать за passed.

### История выполнения (нижние ограничения и паузы не отменяют текущий итог)

- **Пользователь возобновил работу и разрешил закончить ВЕСЬ оставшийся план:** завершить8–9 → реализовать10/11/12, все необходимые исправления, справку, документацию и доступные автоматические проверки; дополнительных подтверждений не ждать. Ручные NVDA/реальныеаккаунты/чистаяVM после сдачи. Пауза отменена этим поручением. Root интегрирует main/Session/PTY и закрывает QA8–9; structured_cli делает10 (opt-in Codex App Server transport/model/native frontend), ux_models делает11 (settings/shortcuts/preferences/workspace/folderpicker/SSHhint), release_tools делает12 (ZIP+реальныйinstaller+CI+install/uninstall+release docs). Чужой dirtydiff сохраняется, commit/push/внешние публикации не разрешены. Source integration/auth/model requests не выполнять автоматически; no autoapprove. Старые ограничения и проверки ниже — история, точный текущий итог дополняется после каждого runnable checkpoint.

- **Работа над 8+9 приостановлена по явной просьбе пользователя «тормозни чуток»:** новых правок Rust/запусков не делать до продолжения. Все собственные QA-процессы закрыты. Последний полноценный кандидат `target/release/bks-app.exe` и `target/stage9/bks-app.exe` — 1289728bytes,18:28:27,SHA256 E481584F2327E39D3E371577C72E10778AF222949A9715FDB36EE0D189DC7186; по нему ранее247 tests/fmt/clippy/release passed. GUI Tui:5passed/1failed (`target/stage89-check/all-20261026-183012-c2b3b099/`) подтвердил неполный context (не было title/options). Root уже исправил context в tui_dialog.rs и усилил основной тест; **эта последняя правка ещё не вошла в EXE:** fmt/clippy прошли, последний cargo test остановился app193passed/1failed — `unicode_path_and_second_selection_are_preserved` всё ещё ожидает path в начале context, теперь там Folder access. При продолжении сначала обновить этот assertion под полный context, выполнить обязательные cargo-проверки/release, затем скопировать свежий checkpoint после закрытия GUI. Остальные предыдущие247passed не выдавать за результат последней правки. GUI Notifications:4passed/0failed/1error timeout (`target/stage89-check/notifications-20261026-183122-72adc1f3/`); evidence foreground=false + startup explicit announcement pause1s, while fixture ждала800ms/отправляла rows сразу. Это пока не подтверждённый productbug; нужно исправить условия eligibility/harness без слепого увеличения thresholds. Повторных GUI послеSTOP не было. Native QA и live Codex/NVDA приёмка ещё не закрыты; живой trustcapture остаётсяunverified. Пользователь запросил обзор оставшегося плана.

- **Этапы 8+9: код интегрирован, финальная native QA выполняется.** Три режима речи сохраняются в `%APPDATA%/BKS Term/settings.json`, Important по умолчанию. Нативный диалог — только Codex CLI0.164.0 startup Folder access, отдельный проверенный native-профиль; Ctrl+Shift+D, полный read-only контекст, native список и explicit Apply/Close. Без автофокуса/подтверждений. Ревью нашло4 дефекта: фоновые строки при возврате, быстрый full-screen scroll без overlap, race output-check→input-enqueue, ownerinput без новойVTrevision. Исправлены destination baseline, linear/rewrite metadata и проверенный scroll suffix, atomic OutputBuffer→InputQueue guard, input_revision. Исходные reviewers подтвердили закрытие замечаний точечно, нового полного review не было. fmt/clippy/247 workspace tests/release passed; `target/release/bks-app.exe` и `target/stage9/bks-app.exe` 1289728bytes,18:28:27. Source fixtures закреплены официальнымtag, живой trustscreen не захвачен; никаких реальных approve/model requests агент не выполнял. Native QA только ownmockexe/APPDATA, речь NVDA остаётся человеку. Известные ранее SSH STATIC hint и Git supplementary input не исправлялись в этом объёме.

- **Активная реализация этапов 8 и 9 разрешена пользователем одним поручением:** без остановок между этапами, ручная приёмка после обоих. Этап 8 реализован: три сохранённых режима Important(default)/All/Manual, bounded/debounced UIA output speech с подавлением redraw/progress/background replay, явные действия приоритетны, live_snapshot независим от истории. fmt/clippy/212 workspace tests/release passed; checkpoint `target/stage8/bks-app.exe` 1117696 bytes, 18:10:30. Этап 9 в интеграции: один конкретный CLI Codex 0.164.0, startup Folder access, exact recognizer + stale guard + native read-only context/list/buttons. Источник fixtures — официальный tag rust-v0.164.0, живой trust capture не подтверждён; попытка запуска capture получила automatic rejection «blocked by policy». Не выполнять реальные варианты/Enter автоматически. Реальные CLI/API/model requests не нужны для приёмки кода. Коммит/push не разрешены; старые незакоммиченные изменения сохраняются.

- **Ручной прогон Этапа 7 остановлен по просьбе пользователя:** стартовый фокус в поле «Команда» пользователь подтвердил; раннее предположение о попадании в журнал отменено его уточнением. После `[Console]::Write('BKS7_FINAL'); exit 7` NVDA произнесла «Команда отправлена» и одно «Windows PowerShell: сессия завершена, код 7» — passed человеком. Ctrl+Up прочитал `BKS7_FINAL` из журнала завершённой вкладки — passed человеком. Повторный тест перезапуска пользователь попросил пропустить как уже пройденный; нового результата по нему не получали. В новом редакторе SSH объявлен «SSH-профиль Новый SSH-профиль», все поля читаются, Tab/Shift+Tab работают — passed по отчёту пользователя. **Найден дефект:** нижняя подсказка «Алиасы обрабатывает OpenSSH…» не читается при обходе формы; код подтверждает STATIC 4206 без Tab stop и связи с полями. Фиксируем для доработки после текущего прохода; Rust не менялся. Пользователь подтвердил доступное сообщение об ошибке порта 70000, затем успешное сохранение `BKS_SSH_TEST` с адресом `bks-test.invalid` и портом 2222, закрытие редактора и объявление сохранения — passed человеком. Следующий предложенный тест повторного открытия/выбора сохранённого профиля не выполнен: пользователь остановил тесты и запросил обсуждение следующих доработок. Docker и WSL установлены по уточнению пользователя; ранее не проверены именно рабочие сценарии/состояние daemon и дистрибутивов. При возобновлении продолжать новые сценарии с повторного открытия SSH-профиля, без повторения принятой ранее функциональности.

- **Код трёх разрешённых итераций Этапа 7 реализован:** ограничение памяти/нагрузки вывода → жизненный цикл сессий → создание/редактирование SSH-профилей и проверки доступных окружений. Пошаговая ручная проверка остановлена пользователем для обсуждения доработок; подтверждённые сценарии и дефекты записаны выше, полная приёмка ещё не закончена. Новая реализация этим обсуждением не разрешена. Класс M; использованы доступные агенты сессии (Kimi/k3 здесь недоступны), обязательные cargo-проверки каждой итерации и один независимый проход ревью. Коммит/push не разрешены. Проводник/выбор рабочей папки остаётся записанной идеей вне реализации. Известный остаточный дефект ввода supplementary Unicode в Git Bash описан ниже и в основном плане.
- **7.1 завершена, NVDA вручную ещё не проверена:** bounded PTY output 1 МиБ + drain 64 КиБ; журнал ограничен 512 КиБ/6000 LF с trim до 384 КиБ/4000 LF, индексы команд до 4096. Модель/нативное представление ребазируют UTF-16 selection/viewport/блоки, удалённая позиция ограничивается началом оставшегося текста. SessionManager обрабатывает round-robin максимум 256 КиБ за тик с мягким бюджетом 8 мс. fmt/clippy/174 workspace tests/release — passed, checkpoint `target/stage7/iteration1/bks-app.exe` 922624 bytes, 16:28:48. GUI 19 passed/0 failed/0 skipped/0 errors: `target/stage7-check/output-20261026-163456-7c58d75e/`, поток 30000 строк, 41 WM_NULL samples/max320ms/0timeouts, cap418565bytes/4812LF, Unicode tail/final marker, точные selection/viewport и draft сохранены. Первоначальные 20000 строк дали задержки EDIT 1,1–1,5 с; новый предел дал trim293–389мс. Fixture использует bounded handshake для захвата retained bookmark до продолжения producer. ADR 0005.
- **Доступные окружения проверены для Stage7:** Git Bash 5.3.15/Git 2.55.0 реально установлен; discovery исправлен: известные установки Git раньше PATH, WSL bash aliases пропускаются в PATH, старый встроенный git-bash с неверным alias разрешается заново при запуске без перезаписи JSON. WSL 2.7.12/kernel 6.18 содержит только docker-desktop, обычного Linux-дистрибутива нет. OpenSSH 9.5p2 и Docker CLI 29.7.2 доступны, Docker daemon отсутствует (pipe dockerDesktopLinuxEngine). Установки/обновления/запуски daemon и SSH-подключения не выполнялись. Диагностика: `target/stage7-check/environment-20261026-154925/`.
- **7.2 завершена, NVDA вручную ещё не проверена:** process exit code + асинхронное закрытие HPCON + дочитывание EOF/пустой очереди; sticky failure/reason и одно lifecycle notice, исходный профиль сохранён для явного перезапуска в новой вкладке. Фоновое освобождение own-сессий ограничено 16 заданиями; finish_cleanup после выхода GUI. fmt/clippy/185 workspace tests/release passed; checkpoint `target/stage7/iteration2/bks-app.exe` 984064 bytes, 16:38:51. GUI **16 passed/0 failed/0 skipped/0 errors**: `target/stage7-check/lifecycleonly-20261026-164241-58ad8326/`; final tail без LF + код 7, однократные journal/status/tab/UIA Notification, новая независимая сессия по 60008, сохранённые старые вкладка/вывод/черновик, live close21ms. Собственные тестовые процессы закрыты.
- **7.3 реализована:** редактор SSH-профилей (меню 60009), системный ssh.exe, атомарное сохранение alias/user/port/key path; защищённый ES_PASSWORD (60010), callback в исходный SessionId напрямую через write, без draft/blocks/JSON. Git Bash legacy alias корректируется при runtime-разрешении пути. Справка F1/README/ADR0005 обновлены. fmt/clippy/194 workspace tests/release passed; итоговые `target/release/bks-app.exe` и `target/stage7/iteration3/bks-app.exe` идентичны SHA256 14AE8E8B388EF1DA1FD7284508267A79C635C759B05360995F97DA5C61D1A272, 1063424 bytes, 16:52:35. Review P2 portable Git после WSL alias в PATH исправлен, regression passed и reviewer подтвердил закрытие; дополнительных P1/P2 нет. Profiles GUI **20 passed/0 failed/0 skipped/0 errors**, `target/stage7-check/profilesonly-20261026-165627-192e62fb/`: create/edit/reload/cancel/invalid-port, ES_PASSWORD/UIA IsPassword, cancel→empty→direct exact UTF8 canary, draft/JSON/journal без canary. Никаких SSH-соединений не запускалось.
- **Финальная совместная нагрузочная проверка:** `target/stage7-check/outputonly-20261026-170308-f7a0602f/`, **19 passed/0 failed/0 skipped/0 errors** на итоговой 7.3. Поток 30000 строк, 41 WM_NULL sample/max533ms/0timeouts при пороге1000ms; journal456497bytes/5248LF; Unicode tail, точные selection/viewport и draft сохранены. Вместе с выбранными lifecycle/profile/Git прогонами итог native/UIA **63 passed/1 failed/0 skipped/0 errors**. Все собственные тестовые процессы закрыты. Полные сценарии — `docs/manual-stage7.md`; GUI протоколы не заменяют речь NVDA.
- **Остаточный дефект Git Bash зафиксирован, не объявлен passed:** `target/stage7-check/gitbashonly-20261026-170131-49a479ad/`, **8 passed/1 failed**. Реальный Git Bash выполняет pwd/git --version; кириллица/CJK/combining в literal команде верны, supplementary символ превращается в U+FFFD. Native command EDIT содержит правильную UTF-16 пару; ASCII-generated printf выдаёт правильные glyph и hex f09f9880. Потеря находится на пути передачи/обработки ввода через ConPTY к Git Bash, конкретный компонент не изолирован. Записано отдельной узкой задачей основного плана; расширения на замену ConPTY/MSYS не делались. Remote SSH/auth/sudo/TUI, обычный WSL и Docker daemon остаются непроверенными по доступности среды.

- **Повторная ручная приёмка новой сборки:** подтверждение отправки после `echo BKS_TEST_2` → Enter — **passed по отчёту пользователя** («команду услышал»). Ctrl+F → `bks_test_2` без Enter → пауза: подтверждено «1 из 2»; F3 из запроса: «2 из 2» — **passed человеком**. До Enter native selection не меняется намеренно для уменьшения лишней речи; после Enter F3 сопровождается выделением и счётчиком.
- **Enter в результат новой сборки — passed человеком:** контрольный speech log показывает запрос `bks_test_2`, счётчик «2 из 2», циклический переход «1 из 2 — с начала», снова «2 из 2», затем Enter → «Экран терминала, редактор, только чтение, выделено BKS_TEST_2» → «2 из 2». NVDA+Tab подтверждает «Экран терминала, редактор, в фокусе, только чтение, многострочный, выделено BKS_TEST_2». Сброс индекса по Enter не подтвердился; раннее сообщение о тишине не считать доказательством поломки перехода.
- **Отсутствие результатов новой сборки — passed человеком:** на запрос `bks_no_match_90827` пользователь подтвердил «Ничего не найдено» и до Enter, и после Enter. Основные речевые исправления (принятие команды, счётчик после ввода и F3, отсутствие результатов) услышаны пользователем. Полная ручная приёмка остальных сценариев Этапа 6 этим не объявляется завершённой.
- **Обязательный выбор профиля по Ctrl+Shift+T — passed человеком:** открытие меню без создания вкладки, Esc с возвратом в прежнюю область и создание вкладки после выбора Command Prompt подтверждены пользователем. NVDA+Tab сообщает «Команда, редактор, в фокусе». Команда `ver` → Enter → Ctrl+Up прочитала `Microsoft Windows [Version 10.0.26200.9711]` и приглашение CMD с текущей папкой `target/release` — выполнение команды выбранным CMD и чтение результата **passed человеком**. Ctrl+Shift+Tab → NVDA+Tab вернул в PowerShell с сохранённым поиском, пользователь подтвердил «всё норм». Затем пользователь запросил обсуждение следующего технического этапа; новых ручных шагов пока не назначаем.
- **Идея пользователя записана в основной план:** открытие BKS Term/новой вкладки в нужной рабочей папке через стандартные средства Windows. Каноническая запись — `docs/setup-and-iterations.md`, Этап 11. Системный выбор папки при создании вкладки и пункт Проводника записаны как варианты; конкретный вход/приоритет ещё не выбраны, реализации пользователь сейчас не поручал. Текущую ручную приёмку продолжаем.

- **Текущая работа после ручной приёмки, 26.10:** пользователь изменил контракт Ctrl+Shift+T (всегда выбор профиля как «+»), затем явно поручил доделать **все** найденные замечания. Прежнее указание только фиксировать проблемы и не менять код этим отменено в пределах перечисленных доработок. Подготовлены подтверждение принятия команды, spoken count/no-results, сокращение лишней речи поиска, актуальное выделение по Enter/Tab и единый profile picker. Нативные EDIT, блоки, текущие пользовательские сессии сохранены.
- **Доработки реализованы в новой сборке:** `announcements.rs` — постоянный visible STATIC 1026 с собственным UIA provider и Notification Other/MostRecent/activity `bks.feedback`, без смены фокуса и отдельного TTS. «Команда отправлена» только после успешного submit; busy/no-session — отдельный короткий статус. `feedback.rs` — debounce 350ms, одно сообщение на явное изменение query, немедленный статус по F3/buttons/Enter, отмена stale query speech при уходе/submit, без timer flood. Во время запроса native VT text/caret не переставляются по каждой букве; Enter/Tab в reader обновляют текущий документ и выделяют current, одинаковый EM_SETSEL повторно не отправляется. Справка/README/manual/ADR обновлены.
- **Проверки доработок завершены:** fmt --check, clippy -D warnings, 161 тест (126 app + 3 writer + 6 PTY + 26 VT), release — passed. Однопроходное независимое ревью нашло 2 P2 (busy отмена pending speech, Tab→current selection), оба исправлены и точечно перепроверены. Финальная сборка `target/release/bks-app.exe` 904704 bytes, 14:19:50; checkpoint `target/stage6/feedback/bks-app.exe` идентичен по SHA256. GUI: **73 passed / 2 skipped / 0 failed / 0 errors** на этой сборке — 44 в `target/stage6-check/announcements-20261026-142111-c1d2a344/`, 29+2 skipped в `target/accessibility-check/run-20261026-142129-4f8e30f4/`. Пропуски: native EDIT не поддерживает ValuePattern, TextPattern прошёл. Получены **11 настоящих UIA Notification events** Other/MostRecent/`bks.feedback`; проверены submit ack, один debounce на burst, count/no-results, сохранение фокуса/quiet native query, Tab/Enter/F3 selection, отсутствие лишних событий при passive output, profile picker cancel/draft/selection/focus и создание сессии. Оба тестовых процесса закрыты. Полная повторная приёмка речи NVDA ещё не завершена; UIA/native проверки не закрывают NVDA автоматически.
- **Исправлен пойманный запуском дефект:** первая сборка уведомлений завершалась до HWND с 0xC0000139 — статический импорт GetWindowSubclass отсутствует в фактической comctl32.dll 5.82. Диагностика — `target/stage6-check/announcements-20261026-141342-a5f06ea5/` (dumpbin/exports/startup exit). В `announcements.rs` lookup заменён на SetPropW/GetPropW/RemovePropW из User32, очищение до освобождения WindowState; targeted review новых lifetime/error paths — без подтверждённых дефектов. После исправления обязательные проверки повторно прошли, запуск и UIA events подтверждены итоговыми GUI прогонами выше.
- **Старые окна:** пользовательский PID 13296 не завершался; прежний EXE сохранён в `target/release/bks-app.before-profile-picker-a28dad42b99e4706ab14c0be06d587d9.exe`, picker-only промежуточный EXE — `bks-app.before-feedback-61396a0ee55a48048766617b8cedf946.exe`. Уже запущенное окно продолжает выполнять старый код; для ручной проверки нужен новый запуск release.

- **Ручная проверка поиска с NVDA продолжается:** Ctrl+F открывает «Поиск по экрану терминала»; в speech log выделялся `BKS_TEST_1`. Переход к следующему совпадению по F3 — **passed человеком** («да, работает»). Shift+F3 ещё отдельно не проверен. Уточнён контракт: Enter переводит к текущему результату и фокусирует VT-документ, F3/Shift+F3 выбирают следующее/предыдущее совпадение. Отсутствие нового совпадения по Enter не является дефектом навигации; тишина и фактический перенос фокуса по Enter, а также удержание фокуса в запросе пока отдельно не подтверждены.
- **Замечание старой сборки — счётчик поиска не озвучивался:** после успешного перехода F3 пользователь подтвердил, что нет сообщений «1 из 2», «2 из 2», поэтому непонятны количество вхождений и текущая позиция. Ручной тест старой версии — **failed**. В новой сборке добавлены краткие UIA Notification с номером и общим количеством после явных действий; доставка событий и отсутствие timer flood проверены автоматически. Голосовую обратную связь NVDA требуется повторно принять человеком.
- **Замечание старой сборки — отсутствие результатов не озвучивалось:** на шаг Ctrl+F → запрос `bks_no_match_90827` без Enter пользователь ответил «молчит». Ручной тест старой версии — **failed**. В новой версии «Ничего не найдено» передаётся настоящим UIA Notification с сохранением фокуса запроса; событие проверено автоматически, слышимость NVDA ожидает ручной приёмки.
- **Замечание к многословности поиска:** пользователь отметил «всё прочитало», в озвучку попадают prompt, команды и результаты; детали речи явно отложены до последующей доработки.
- **Ручное закрытие поиска — passed человеком:** на шаг Esc → NVDA+Tab (закрытие поиска и возврат фокуса в исходный элемент) пользователь ответил «всё работает».
- **Ручная независимость вкладок до доработок — passed человеком:** в исходной вкладке оставлен открытый поиск `bks_test_1`; прежний Ctrl+Shift+T создал новую вкладку с закрытым поиском и фокусом в пустом поле «Команда». Затем Ctrl+Shift+Tab → NVDA+Tab восстановил открытый поиск и запрос исходной вкладки, пользователь подтвердил «всё верно, отработало корректно». После этого пользователь потребовал выбор профиля при каждом Ctrl+Shift+T, реализовано отдельной доработкой выше.

- **Ручная приёмка NVDA 26.10 начата, идём по одному шагу:** начальный фокус **passed человеком** — после Alt+Tab в BKS Term и NVDA+Tab озвучено «Команда, редактор, многострочный, пусто». Полная NVDA-приёмка Этапа 6 ещё не пройдена.
- **Замечание старой сборки — нет речевого подтверждения отправки команды:** пользователь ввёл `echo BKS_TEST_1` и нажал Enter; результат появился, но непосредственно при Enter NVDA молчала. Ручная проверка обратной связи старой версии — **failed**. Сначала пользователь разрешал только фиксацию; последующим поручением разрешил исправить всё. В новой сборке успешное принятие команды сопровождается «Команда отправлена» через UIA Notification без потери фокуса ввода; доставка события проверена автоматически, речь NVDA нужно повторно проверить человеком.
- **Наблюдения того же теста:** Shift+Tab переводит в журнал/к результату. Это само по себе не подтверждённый дефект: переход в предыдущий контрол соответствует текущему порядку Tab и не заменяет подтверждение отправки. В speech log повторяются prompt, предупреждение PowerShell об отключении PSReadLine и участки вывода; причина повторов пока не установлена. Фокус непосредственно после Enter отдельно не подтверждён: в присланном логе есть имена и поля команды, и журнала.
- **Ручная проверка переходов к блокам — passed по сообщению пользователя:** после шага с Ctrl+Up пользователь подтвердил переходы между результатом `echo BKS_TEST_1` и выводом `docker ps`; NVDA читает соответствующие фрагменты журнала. Ошибка подключения Docker записана как вывод дочерней команды, исправление Docker не запрашивалось. Возврат из журнала в поле «Команда» через Ctrl+Shift+Space, затем проверка NVDA+Tab — **passed человеком** («да, это работает»). Дефект речевого подтверждения Enter остаётся открытым, код и EXE не менялись.
- **Ручная проверка открытия VT-документа — passed человеком:** меню Ctrl+Shift+P → «Читать экран терминала» → NVDA+Tab озвучивает «Экран терминала, редактор, в фокусе, только чтение, многострочный»; в speech log есть предупреждение оболочки, команды echo/docker, их результаты и prompt. Пользователь подтвердил «вроде тоже норм» и запросил объяснение назначения отдельного источника. Пройдено именно открытие/имя/роль/чтение содержимого; устойчивость чтения и поиск с NVDA ещё не проверены. Сейчас объясняем различие журнала потока и актуального VT-документа, следующий ручной тест пока не назначен.

- **Этап 6, текущая работа по согласованным трём итерациям:** пользователь разрешил доступный VT-документ → устойчивое чтение → поиск, строго последовательно, с субагентами. Итерация 1 реализована и сохранена в `target/stage6/iteration1/bks-app.exe`: 122 теста/fmt/clippy/release passed. Добавлен отдельный native EDIT 1018 «Экран терминала», выбор через «Читать экран терминала»/«Читать журнал команд»; default поле+старый журнал сохранены. `ReadingDocument` экспортирует history/main или только alt, UTF-16→cell mapping и softwrap, не меняя viewport. UIA-скрипт подтвердил историю/длинную строку в новом TextPattern (6 passed), затем focus-case прервался внешней сменой foreground (error/skipped, не passed); NVDA не проверялась.
- **Итерация 2 завершена:** `reading_state.rs` (13 тестов) хранит main/alt закладки отдельно, сохраняет выделение и логический viewport, нормализует Unicode; `native_text.rs` применяет частичную замену текста. Позиции обоих источников сохраняются как UTF-16 offsets, до resize выполняется захват позиции. Автопрокрутка — collapsed caret в конце + viewport внизу; пустой документ сохраняет прежний follow и не включает его сам. Ctrl+Shift+Space меняет фокус input↔выбранный reader либо graph↔reader в direct mode, capture снимается. 138 тестов/fmt/clippy/release passed; checkpoint `target/stage6/iteration2/bks-app.exe`. GUI: 70 passed / 0 failed / 0 skipped / 0 errors (`target/stage6-check/iteration2-20261026-110705-27ee2b10/`), включая очистку→append, main/alt, resize, вкладки и обе ветки переключения фокуса. NVDA не проверялась.
- **Итерация 3 завершена по коду:** `search.rs` — буквальный Unicode lowercase-поиск по VT-документу, точные исходные UTF-16 ranges, инкрементальный пересчёт suffix+overlap и сохранение current. `search_bar.rs` — native query/buttons/status, hidden default, adaptive layout. Ctrl+F/menu 60007 открывают поиск; F3/Shift+F3 циклические, status сообщает «с начала/с конца»; Enter в запросе открывает native VT с выделением; Esc/Close возвращают source/control. Query/current/open независимы по вкладкам, capture снимается. Из графики остаётся видимым overlay до Enter, подсветка всех видимых matches/current не меняет ANSI. Ctrl+Up/Down закрывает поиск при переходе к прежним блокам. F1/README/manual обновлены. 157 тестов (122 app + 3 writer + 6 PTY + 26 VT), fmt/clippy/release passed; EXE `target/release/bks-app.exe` и `target/stage6/iteration3/bks-app.exe`. Однопроходное независимое ревью интеграции: подтверждённых дефектов нет.
- **GUI итерации 3:** `target/stage6-check/iteration3-20261026-113408-125b1088/` — 33 passed / 0 failed / 3 skipped / 1 environment error. Passed: native UIA names/read-only/selection, Unicode в истории, count/wrap/empty/no-results, запрос не отправляется в PTY и сохраняет draft, Enter выбирает исходный UTF-16 фрагмент, очистка VT убирает результаты. До Enter прошли guarded waits реальных Ctrl+F/F3/Shift+F3/Tab/Shift+Tab. Затем внешняя потеря foreground: Esc, скрытый Tab, Ctrl+Up/Down из search result и per-tab GUI остались **не проверено**, не passed. PrintWindow wide/narrow/graph/graph-closed просмотрены: строка без перекрытий, видимые matches/current различимы, после закрытия вернулись ANSI цвета. NVDA/настоящие системные DPI/High Contrast проверяет человек. Старый GUI fail Enter был неверным сравнением с pre-resize raw-журналом: ConPTY repaint увеличил его 1925→3281; финальные before/after Enter равны 3281 и выделение [606,618] верно. Ранний HWND0 в другом прогоне был гонкой инициализации harness, исправлен ожиданием Execute 1003; приложение не падало. Дальнейшие перезапуски GUI прекращены.
- **Текущий этап:** пользователь 26.10 подтвердил, что доступность появилась после возврата нативных контролов, и сообщил о пропавшей навигации Ctrl+Up/Down по блокам. Исправлены обе причины: Ctrl+Up больше не застревает на последнем результате, Ctrl+Down назначен на следующий; отправки через поле команды без OSC 133 создают блоки. Первая графическая сборка Этапа 5 остаётся отклонённой; восстановление стартового сценария не означает приёмку всех NVDA/DPI/TUI-сценариев. VT-модель/рендер сохранены как явный режим; полная доступная VT-проекция и поиск остаются Этапом 6.
- **Навигация по блокам 26.10:** из поля команды Ctrl+Up открывает последний блок, Ctrl+Down — первый; в журнале Up идёт к текущему/предыдущему началу, Down — к следующему, края ограничены первым/последним блоком. Alt+Up/Down сохранены как синонимы. `Session::submit_command` регистрирует блок только после принятия ввода очередью. Без OSC граница берётся из UTF-16 длины журнала при явной отправке, при совпадении эха начало уточняется; если эхо изменено/отсутствует, граница может включать команду/prompt. Старый точный OSC-путь сохранён. Для generic `text_updated` вызывается по новым байтам, без постоянного timer lookup. Прямой Enter по-прежнему не создаёт блоки. fmt/clippy/test/release passed, 112 тестов (86 app + 3 writer + 6 ConPTY + 17 VT); справка и README обновлены. Старый пользовательский процесс оставлен, EXE сохранён как `bks-app.before-block-navigation-*.exe`.
- **Регрессия навигации, проверка окна:** `scripts/check-accessibility.ps1 -BlocksOnly` в PowerShell — 79 passed / 2 skipped / 0 failed (`target/accessibility-check/run-20261026-095002-c407152e/`). Проверены три различных UTF-16 смещения, Up/Down в обе стороны, крайние блоки, переход из середины к началу, Ctrl+Down из поля, Alt-синонимы, сохранение черновика и независимость вкладок. Следующие прогоны с BM_CLICK и разнесёнными modifier key events подтвердили переходы и края, но прерывались PID-guard при потере foreground (environment error, не passed); дальнейшие попытки остановлены. GUI-клавиши в CMD после этой правки **не проверено**; отправки и навигационная модель CMD проверены реальными ConPTY unit/integration сценариями в cargo test. Речь NVDA при новых переходах проверяет пользователь.
- **ТОЧКА ОСТАНОВА 26.10, исправление после обратной связи:** `target/release/bks-app.exe` пересобран. По умолчанию видны нативные поле «Команда» и журнал, фокус в поле команды. Tab/Shift+Tab обходят видимые активные контролы и реальные вкладки в обе стороны. «Прямой ввод» в меню включает VT; Tab остаётся навигацией, пока пользователь явно не включит передачу Tab через Ctrl+Shift+I / меню. F1, Ctrl+Shift+Space и уход фокуса снимают захват. Для native controls имена задаются через оба GUID: MSAA PROPID_ACC_NAME и UIA Name_Property_GUID. CMD/Git Bash без OSC 133 больше не блокируются ожиданием интегрированного prompt в поле команды. fmt/clippy/test/release passed; 103 теста (77 app + 3 writer + 6 ConPTY + 17 VT). Прежние 22 GUI-checks не проверяли доступный текст/Tab и не закрывают отрицательную пользовательскую приёмку. Старый запущенный пользователем EXE сохранён как `target/release/bks-app.before-accessibility-fix-*.exe`; его процесс не завершался, для проверки нужен новый запуск.
- **Проверки исправленной release 26.10:** `scripts/check-accessibility.ps1` — PowerShell 206 passed / 4 skipped / 0 failed (`target/accessibility-check/run-20261026-092632-e7389005/`), CMD 205 passed / 5 skipped / 0 failed (`run-20261026-092645-fd5ed1cd/`). Подтверждены реальные UIA Name/Document/TextPattern/read-only/selection, Unicode-редактирование и команды, циклы Tab/Shift+Tab, динамические вкладки, F1 и захват/освобождение Tab. Native multiline EDIT не предоставляет ValuePattern, чтение и выделение проверены через TextPattern. Обновлённый `scripts/check-stage5.ps1` — 25 passed, включая обе ветки закрытия вкладки. NVDA-речь не проверялась, отрицательная пользовательская приёмка требует повторной проверки.
- **Нюансы исправления:** IAccPropServices удерживается до завершения COM UI-потока; native STATIC-метки перед EDIT обеспечивают устойчивые UIA-имена журнала/команды/справки. Добавлены Ctrl+A в обоих полях и справке, явный Tab/Shift+Tab в справке (одного IsDialogMessageW для обычного owned window было недостаточно). В UIA-harness нужны штатные Framework Win32 proxies и compiled C# инициализация; ранние Pane/no-patterns были ошибкой клиента проверки. Графический VT остаётся без полноценного текстового провайдера, его UIA.SetFocus не поддерживается; клавиатурный opt-in сценарий проверяется через Win32, не объявляется доступностью NVDA.
- **ТОЧКА ОСТАНОВА 25.10:** после снимка 19:01 и уточнения об одном действии «Новая вкладка» шапка уменьшена на 40% до 36 DIP (логотип 24, подпись 14 DIP), градиент усилен, меню профилей теперь тёмное нативное HMENU с доступными именами/отметкой режима. Вверху только один `+`: открывает только профили, выбор создаёт реальную вкладку; стрелка удалена. Прямой ввод доступен через sidebar «Профили» / Ctrl+Shift+P; Ctrl+Shift+T быстро создаёт default-профиль. fmt/clippy, 66 тестов, обычный `cargo build --release` и 24 проверки живого release-окна — passed. Обновлён `target/release/bks-app.exe`, новое окно открыто; артефакты — `target/design-review/compact-menu/`. NVDA, реальные системные DPI и High Contrast ещё ожидают ручной приёмки.
- **Поиск — только в плане (25.10):** пользователь попросил описать поиск по выводу, подсветку совпадений и перенос фокуса, затем явно отложил реализацию. В `docs/setup-and-iterations.md` версии 0.3 добавлена отдельная итерация Этапа 6: Ctrl+F, F3/Shift+F3, Enter к результату, Esc обратно, подсветка всех видимых совпадений и текущего, независимое состояние вкладок, Unicode/UTF-16, доступность и защита от отправки поисковых клавиш в ConPTY. Код поиска не менялся; продолжение разработки остаётся Этапом 5.
- **ТОЧКА ОСТАНОВА 24.10:** полный набор проверки профилей/сессий/вкладок прошёл (50 unit + 4 PTY integration = 54). Прямой Unicode-ввод и фоновый вывод независимых вкладок проверены в реальном окне. Автоматический выбор пункта меню профиля и подтверждение закрытия вкладки остаются непроверенными из-за нестабильной инъекции; ручные NVDA/DPI отложены пользователем и не считаются пройденными.

## 1.1. История состояния bks-pty (актуальный итог — в начале файла)

- `crates/bks-pty/src/lib.rs` — собственная обёртка ConPTY на `windows-rs 0.62.2`; выбор зафиксирован в `docs/decisions/0001-conpty-engine.md`. Исправлен запуск дочернего процесса: `STARTF_USESTDHANDLES` и недействительные stdio handles не дают выводу утечь в родительскую консоль.
- Зависимости bks-pty: production — только `windows 0.62.2`; `portable-pty 0.9.0` — dev-зависимость для spike.
- `crates/bks-pty/tests/conpty.rs` — 6 интеграционных тестов: прежние 4 плюс FIFO очереди ввода и закрытие с большим непотреблённым вводом; **passed 26.10**. `input_writer.rs` добавляет 3 unit-теста, включая отмену заблокированного pipe и освобождение duplicate handle.
- `PtySession::input_writer()` создаёт отдельный FIFO writer: очередь до 2 МиБ/256 сообщений, `write` принимает сообщение целиком либо возвращает WouldBlock, ошибки worker доступны через `check_error`. Синхронный `write_input` сохранён. В приложении writer освобождается до PTY; Drop отменяет очередь и повторяет отмену I/O до завершения worker, абсолютный deadline ОС не обещан.
- `crates/bks-pty/examples/spawn_debug.rs` — отладочный пример (команда аргументом).
- `crates/bks-pty/examples/pty_spike.rs` — **spike portable-pty завершён успешно**: маркер получен через PTY, `cmd.exe` сам завершился с кодом 0. Чтение вынесено в поток, есть общий таймаут 10 с; на стартовый запрос `ESC[6n` пример отвечает `ESC[1;1R`. Проверка: `cargo run -p bks-pty --example pty_spike`.
- Рабочая директория: `C:\work\BKS.Lab\python\my_projects\BKS-Term` (git-репозиторий).

## 1.2. История состояния bks-app (актуальный итог — в начале файла)

- `crates/bks-app/src/main.rs` — Win32-окно с нативными полем команды и read-only EDIT журнала по умолчанию. VT-поверхность открывается явно. `keyboard_navigation.rs` задаёт порядок Tab/Shift+Tab, исключает системный Alt+Tab и Ctrl+Tab из захвата. Нативные EDIT уведомляют о фокусе, чтобы чтение/ввод восстанавливались правильно после переключений. Динамические вкладки создаются через `+`/меню профиля, закрываются с подтверждением, переключаются мышью, Tab, Ctrl+Tab/Ctrl+Shift+Tab и Alt+1…9. Экран, режим чтения, selection/draft/позиция чтения независимы по сессиям.
- Визуальная доводка 25.10: фирменная шапка 36 DIP с существующим логотипом 24 DIP и подписью 14 DIP и кнопками окна; цельные тёмные вкладки с иконкой, зелёным контуром активной вкладки и встроенным крестиком; единственная кнопка `+` с выбором профиля; sidebar 144/56 DIP; статус 40 DIP с точкой, текстом соединения, UTF-8 и реальным размером PTY. Переключатель «Прямой ввод» доступен в меню sidebar «Профили» (Ctrl+Shift+P); верхний `+` содержит только выбор профиля. Командный редактор сохранён, уменьшен до 44 DIP; в прямом режиме скрыт. Вкладки и действия сохраняют правильный порядок перекрытия при ресайзе и смене режима.
- `window_frame.rs` сохраняет системные команды окна, hit targets перетаскивания/resize/maximize при собственной шапке; `control_state.rs` обеспечивает hover нативных owner-draw кнопок через subclass. Доступные имена сохранены. Исправлены потеря черновика при `+`, обновление названия соединения при смене вкладки, восстановление геометрии режима каждой вкладки и стартовый resize до расчёта размеров (не передавать PTY сетку 1×1 из ещё пустого контрола).
- `profile.rs` — PowerShell/CMD + обнаружение pwsh/Git Bash/WSL при наличии, Windows-quoting аргументов, валидация, версионированный JSON в `%APPDATA%\BKS Term\profiles.json` с атомарной заменой через `MoveFileExW`. Повторное сохранение и `default_profile_id` проверены. Generic SSH-профиль не создаётся без реального хоста.
- `session.rs` — каждая сессия владеет своими PTY, буфером, OSC-парсером и CommandBlocks. `SessionManager` опрашивает все сессии, создаёт/активирует/закрывает их без смешивания буферов. `terminal_input.rs` кодирует спецклавиши и UTF-16/surrogate pairs; прямой Unicode-ввод в активную PTY проверен в окне. Resize вычисляется по метрикам шрифта и передаётся в ConPTY.
- `command_blocks.rs` и `terminal_output.rs` пока используются для старого текстового журнала/OSC 133. Навигация Ctrl+Up, Alt+Up/Down, Ctrl+Shift+C, быстрый Enter и вывод 104925 символов проверены. `check-output-view.ps1` подтверждает selection/follow.
- `check-direct-input.ps1` подтвердил прямой текст/Enter/Unicode; `check-tabs-basic.ps1` подтвердил независимые вкладки, фоновый вывод и Ctrl+Tab. Выбор реального профиля через меню и создание вкладки дополнительно проверены 25.10 в release. Подтверждение закрытия вкладки остаётся непроверенным; закрытие всего окна проверено отдельно и не заменяет этот сценарий.
- `theme.rs` и `system_colors.rs` централизуют палитру и High Contrast через системные цвета Windows. `ansi_palette.rs` хранит 16 цветов/default/cursor/selection для будущего VT. RichEdit-подсветка prompt откачена: у RichEdit внутренние позиции CRLF несовместимы с CommandBlocks/WM_COPY; прежний EDIT восстановлен и проверки снова прошли.
- `crates/bks-terminal` интегрирован в сессии: снимки viewport, курсор, wide/combining, история до 10 000 строк, alternate screen, resize, режимы ввода и очередь VT-ответов. 17 тестов passed 26.10. OSC 52 отключён; распознанные команды/подтверждения не выполняются. `terminal_view.rs` рисует GDI-сетку и цвета; ADR `0003-gdi-terminal-surface.md`. `help.rs` — отдельная modeless справка с native EDIT, F1/Esc/Tab и возвратом фокуса.
- Пользовательский логотип/иконка и release `target/release/bks-app.exe` сохранены. Полный fmt/clippy/test/release набор 24.10: 50 bks-app + 4 PTY + 8 bks-terminal = 62 passed. `capture_window_screen.ps1` используется для визуальной приёмки owner-draw; `PrintWindow` даёт артефакты.
- Известные ограничения: ручные NVDA/DPI остаются непроверенными. Журнал хранит очищенный поток; полноценная VT-проекция и поиск доступны отдельным источником «Экран терминала». SSH-профили/настройки и prompt-подсветка не реализованы. Блоки результатов создаются командным редактором; прямой Enter пока не создаёт блок. Качество сложных emoji зависит от GDI/шрифтов. Ограничена VT-история и очередь ввода; ограничение журнала и очереди вывода остаётся Этапом 7. Без открытого поиска Esc в терминале передаётся процессу и не закрывает окно.

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
- Скрипт: `deepseek_codegen.py SPEC --out OUT [--max-tokens N] [--reasoning-effort none|low|high|max]`, модель `deepseek/deepseek-v4.1-flash-1017` через Polza.ai. По умолчанию reasoning `low` (env `POLZA_REASONING_EFFORT`); в ТЗ обязательна строка `Reasoning: ... — обоснование`, скрипт передаёт `reasoning.effort` и печатает `reasoning_tokens`.
- Для Этапа 2 DeepSeek с `low` сгенерировал `dpi.rs` (4819 completion/2944 reasoning), `layout.rs` (9489/8103) и `theme.rs` (12816/11238), все `finish=stop`; код перенесён вручную с исправлением Markdown-обёрток и сигнатур windows-rs. Локальный mock API подтвердил передачу всех уровней reasoning и статистики; боевые вызовы Polza выполнены успешно.
- Ключ: `POLZA_API_KEY` (задан через setx 23.10). **Нюанс:** сессии Kimi, запущенные до setx, ключ напрямую не видят — инъекция из реестра:

  ```bash
  powershell -NoProfile -Command '$env:POLZA_API_KEY=[Environment]::GetEnvironmentVariable("POLZA_API_KEY","User"); python "C:/Users/user/.kimi-code/skills/deepseek-coder/deepseek_codegen.py" "<spec>" --out "<out>"'
  ```

- Скрипту нужны пути Windows-стиля (`C:/...`), не Git Bash `/tmp`.
- **Правило дробных ТЗ (после инцидента 23.10):** одно ТЗ = один модуль/класс/файл. Два первых запроса с ТЗ «на весь Этап 1» уходили в runaway-генерацию до потолка 131 072 выходных токенов (~18 ₽ за запрос). С 24.10 в скрипте `--max-tokens` по умолчанию **100000**, при обрезании ответа — код выхода 3.
- Черновики DeepSeek складывать в `.deepseek/` (не коммитить; добавить в `.gitignore` при создании workspace).

## 4. Исторические артефакты первых этапов

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

## 9. Следующий шаг после возобновления

Работа приостановлена 26.10 по просьбе пользователя. Сначала закрыть
последний baseline race в native harness preview.2 (точное состояние и
артефакты в начале файла), затем продолжить ручную приёмку NVDA/Kimi по
одному шагу из `docs/manual-final.md`. Справка — F1 и `docs/help.md`,
ограничения — `docs/known-limitations.md`. Старую пользовательскую сессию
не закрывать. Исторический план ниже не разрешает работу во время паузы.

### История прежних планов и визуальных итераций

**Актуально 26.10 после доработок и пошаговой ручной проверки:** пользователь
подтвердил основные речевые исправления, поиск, выбор/отмену профиля,
работу CMD и возврат к сохранённому поиску PowerShell; результаты выше.
Полная приёмка Этапа 6/MVP-A (длительный вывод при чтении, resize, DPI и
прочие сценарии manual-stage6) этим не объявляется пройденной.

Пользователь разрешил три последовательные итерации Этапа 7: ограниченный
вывод → lifecycle/restart → нативные SSH-профили и доступные окружения.
Код всех трёх подключён, для каждой сохранена запускаемая checkpoint-сборка;
итог GUI/ревью и точные проверки — в начале памяти. Следующий пользовательский
шаг — пошаговая ручная проверка свежей итоговой сборки по manual-stage7.
Речь NVDA и удалённый SSH не объявлять проверенными автоматически.
Новых возможностей Этапа 8 до этой проверки пользователь не поручал.
Идея открытия в выбранной папке остаётся отдельным пунктом плана.
Ниже сохранена история решений.

Пользователь добавил `docs/BKS_Term_Профили_вкладки_SSH_ТЗ.md` и `docs/BKS_Term_Design_Spec_and_Reference/BKS_Term_Design/BKS_Term_UI_Design_Spec.md` с PNG-референсом. Они включены в основной план `docs/setup-and-iterations.md` версии 0.2.

Обратная связь 24.10: пользователь условно принял текущую оболочку, но отметил неполное соответствие референсу по цветам: почти чёрный фон и белый базовый текст есть, полноценной цветовой обработки вывода пока нет. Это зафиксировано в основном плане как незавершённая цветовая доводка, не как полное соответствие.

Уточнение пользователя 25.10: внешний вид нужен максимально близкий к PNG-референсу, а не только похожая тёмно-зелёная палитра. Расхождения затрагивают также композицию и визуальную иерархию: ярко-зелёные служебные кнопки и отдельный крестик вкладки, оформление вкладок и заголовка, боковую панель, строку состояния и площадь терминальной поверхности. Сравнивать результат с самим PNG и визуальным ТЗ; текущую оболочку не считать окончательно принятой по дизайну. Изображённый вывод команд не воспроизводить статически и не подменять ANSI раскраской по словам.

Визуальная итерация 25.10 выполнена по утверждённому пользователем плану из пяти пунктов: шапка/компоновка, вкладки, кнопки/меню, sidebar/статус, рабочая область. PNG повторно просматривался на этапах работы; финальные снимки получены с реального экрана, а не через PrintWindow. Внешний вид оболочки приближен к референсу в пределах работающих функций; полного совпадения терминального вывода пока нет. Недостающие разделы и показатели подключать вместе с реальными функциями и достоверными данными.

Изменены `main.rs`, `shell.rs`, `layout.rs`, `theme.rs`, добавлены `control_state.rs` и `window_frame.rs`; в `crates/bks-app/Cargo.toml` включён `Win32_UI_Shell` для native subclass, без новой библиотеки. README обновлён. PTY/VT/ANSI-реализация не изменялась. Обязательные проверки 25.10: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --release` — **passed**. Для release использован `CARGO_TARGET_DIR=target/design`, поскольку прежний EXE открыт пользователем. Тесты: 49 bks-app + 4 PTY integration + 8 bks-terminal = **61 passed**; геометрические проверки объединены в параметризованные для 96/144/192 DPI и добавлен контракт собственного заголовка.

На новой release выполнено **20 passed** проверок живого окна: команды/многострочный ввод, реальные вкладки, сохранение черновика и вывода, восстановление геометрии режимов между вкладками, сворачивание sidebar, размер PTY, minimize/maximize/restore/close; выбор режима через нативное меню с UI Automation, Tab; отсутствие перекрытия `+`/меню после смены режима и resize. Снимки: `target/design-review/shell-final.png` (1492×928, командный режим), `shell-direct.png` (прямой ввод), `shell-compact.png` (480×320). Логи и воспроизводимые PowerShell-проверки находятся рядом. Это не NVDA-приёмка.

После уточнения пользователя о старом виде новая проверенная сборка установлена также в `target/release/bks-app.exe` (равенство SHA-256 проверено). Запущено новое окно без остановки прежних сессий. Файл старой работающей сборки сохранён как `target/release/bks-app.before-design-00f24afff7544e9395e3b5d7591094f4.exe`; исходный процесс продолжает работу до закрытия пользователем. Для последующих запусков обычный release-путь уже содержит новый дизайн.

Повторная обратная связь пользователя по снимку 18:31: предыдущая оболочка недостаточно соответствовала PNG — зеленоватые плоские фоны, отсутствие градиентов шапки, условные зелёные иконки и неполная левая панель. Эти визуальные различия не зависят от VT; не оправдывать ими отложенную доводку оформления. Предыдущую визуальную работу не считать принятой пользователем.

Повторная правка: `theme.rs`/`system_colors.rs` теперь используют `bg.terminal=#030607`, `bg.window=#080D0E`, `bg.toolbar=#0C1112`, `bg.sidebar=#0C1414`, `bg.surface=#0B1112`, измеренные по PNG. Новый `background.rs` рисует градиенты шапки и нижней части sidebar, согласованные между родительским окном и дочерними контролами; High Contrast оставляет системную сплошную заливку. Шапка 60 DIP, логотип 40 DIP, скорректированы отступы sidebar и выделение. Пункт «Профили» с иконкой папки вызывает существующий выбор реальных профилей с привязкой меню к нажатому элементу; пустые SSH/настройки не добавлены.

Новый `profile_icons.rs`: оригинальные ресурсы `git-bash.exe`/`git-for-windows.ico` (именно разноцветный значок из PNG), `cmd.exe`, `wsl.exe` (Tux), кэш HICON на UI-потоке и освобождение ресурсов. Никакой зелёной перекраски. PowerShell сохраняет chevron; иконка Ubuntu используется только при наличии реального установленного ресурса Ubuntu. Автотесты добавлены для классификации профилей и корректного fallback.

Проверки последней правки: fmt, clippy, `cargo test --workspace` (51 bks-app + 4 PTY + 8 bks-terminal = **63 passed**), release — **passed**. В живом release-окне проверены прежние 20 сценариев и три реальных создания вкладок Git Bash/WSL/CMD через sidebar «Профили»: **23 passed**. Выбор профилей проверялся реальным нажатием по контролу/меню с предварительной проверкой принадлежности точки тестовому процессу; одного возврата UI Automation Invoke недостаточно, требуется появление настоящей вкладки. Автоматическая проверка пикселей подтвердила чёрный фон (разница с выбранной точкой PNG по каналам не более 2), наличие градиентов и красных/синих пикселей настоящего значка Git Bash. Скриншоты `reference-release.png` и `reference-compact.png`, логи и результаты — в `target/design-review/reference-match/`.

Последняя сборка установлена в обычный `target/release/bks-app.exe`, SHA-256 совпадает с протестированной; открыто новое окно. Предыдущая работающая версия сохранена как `target/release/bks-app.before-reference-match-615ed6316c1e4d21a5be2e4e3ea7315d.exe`; пользовательские сессии не завершались. При интеграции VT синхронизировать пока неиспользуемый `ansi_palette::DARK_PALETTE.default_bg` (в spike ещё #070B0A) с новым `theme::BG_TERMINAL` (#030607). Терминальное ядро и обработка ANSI этой визуальной правкой не изменялись.

Уточнение по снимку 19:01: верхняя полоса и название слишком крупные; пользователь попросил уменьшить их примерно на 40%, сделать градиент заметнее и оформить белое меню в стиле терминала. Дополнительно пользователь отказался от двух соседних кнопок: нужен один `+`, который сразу предлагает профиль. «Продолжай» сохранило весь этот согласованный объём.

Реализовано: `layout::TITLE_HEIGHT=36` вместо 60, логотип 24 вместо 40 DIP, подпись 14 вместо 22 DIP, кнопки окна 32 DIP по высоте; новый размер согласован с отрисовкой и native hit targets. Градиент шапки усилен, чёрная терминальная палитра сохранена. `profile_menu.rs` оставляет native HMENU/меню Windows, рисует тёмные строки, иконки, hover/выделение и отметку режима, измеряет строки с учётом DPI. Подписи сохранены через native MIIM_STRING и MSAA MSAAMENUINFO; данные в стабильных Box, callback проверяет владение, Drop освобождает меню/кисть. Первая буква/мнемоники поддержаны WM_MENUCHAR; три unit-теста покрывают цикл, escape амперсанда и Unicode. В High Contrast используется стандартное системное меню.

Убраны видимый контрол стрелки и его место в раскладке/Tab-цикле. Нажатие `+` открывает только действующие профили; выбор запускает выбранную сессию. Меню «Профили» слева и Ctrl+Shift+P дополнительно содержат «Прямой ввод». Сочетание Ctrl+Shift+T оставлено для немедленного создания default-профиля. README и память обновлены.

Проверки этой правки: fmt, clippy, **66 passed** (54 bks-app + 4 PTY + 8 bks-terminal), обычный `cargo build --release` — **passed**. В новой release **24 passed** сценария: 11 прежних smoke, 7 проверок единственного `+`/выбора реальной вкладки/режимов/checked state, 5 проверок native MenuItem names и клавиш Up/Enter/Escape/переключения режима, закрытие кнопкой окна. В тестах важно действительно закрывать предыдущее popup (например нажатием вне его): WM_CANCELMODE, отправленный только родителю, не гарантирует завершение popup и давал ложные сбои автоматизации. Диагностическая маршрутизация WM_COMMAND была проверена и убрана; в продукте её нет. Никаких NVDA passed из этих тестов не следует.

Обычный `target/release/bks-app.exe` пересобран и открыт с пользовательскими настройками; тестовые окна и сессии завершены, пользовательские не закрывались. Снимки `compact-profile-menu.png`, `compact-actions-menu.png`, `compact-window-final.png`, проверки и логи — `target/design-review/compact-menu/`. Старый `target/design/release` теперь содержит предыдущую итерацию; запускать текущий EXE из обычного `target/release`.

Для ручной приёмки остаются: чтение шапки/вкладок/меню с NVDA; переключение командного и прямого режима; hover/focus и системное управление окном/Snap; реальные масштабы Windows 100/150/200% и High Contrast. Ни одна из этих ручных проверок не объявлена пройденной. Отдельное поле команды, полноценные ANSI-цвета, курсор и TUI остаются следующей технической итерацией; декоративные SSH/настройки и ложный текущий каталог не добавлялись.

Пользователь явно отложил ручные NVDA и системные проверки 100/150/200% и разрешил переходить к следующим этапам. Они остаются **не проверено**, не **пройдено**, но временно не блокируют разработку.

Техническое продолжение после реализации 26.10 — Этап 6: полный доступный буфер NVDA, синхронизированный с VT, затем запланированная отдельная итерация поиска. До продолжения пользователь проверит текущую сборку вручную. Prompt-подсветку через RichEdit не возвращать без новой модели позиций. Обе ветки подтверждения закрытия вкладки проверены автоматически 26.10; ручная NVDA-приёмка меню и диалога ещё требуется.

26.10 один независимый проход ревью выявил потерю фокуса при восстановлении окна: WM_SETFOCUS выбирал теперь скрытый журнал. Исправлено на общий выбор видимого элемента текущего режима; регрессия проверена в живом окне. Закрытие активной вкладки тоже возвращает фокус видимому элементу. Ошибки запуска/ввода показываются нативным диалогом, а не только в скрытом журнале. Скрипт GUI-проверки использует отдельный APPDATA и закрывает только созданный процесс. Для Unicode-примеров через `[Console]::WriteLine` в Windows PowerShell явно задаётся UTF-8, иначе символы может заменить кодировка самого дочернего процесса.

Дальнейший порядок: профили и независимые сессии с прямым PTY-вводом → подключение настоящих вкладок/sidebar к готовым компонентам и история/черновики для каждой сессии → VT/ANSI на готовой терминальной поверхности → полный доступный буфер NVDA → Git Bash/WSL/SSH/Docker → уведомления, семантические TUI и AI CLI → UX/окна/устойчивость → упаковка.
