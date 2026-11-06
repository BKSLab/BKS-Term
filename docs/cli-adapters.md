# Адаптеры CLI: общий терминал, отдельные стратегии

Срез официальной документации и локальных бинарников: **2 ноября 2026**;
профиль OpenCode дополнительно проверен **5 ноября 2026**,
нативный Codex — **6 ноября 2026**.
Документация описывает публичные режимы и сочетания клавиш; геометрия ячеек
проверяется отдельно по VT-записям. Обновление страницы документации само по
себе не подтверждает совместимость нового renderer.

Общий Win32/native UI, ConPTY и VT-модель остаются едиными. `AdapterKind`
выбирает стратегию проекции composer и разрешённые семантические соглашения
в `tui_adapters.rs`. Это не набор отдельных терминалов и не разрешение
исполнять прочитанный текст. Стратегии не отправляют PTY bytes.

`Auto` определяется владельцем запуска по профилю/принятой команде.
`Shell` и `Universal` используют общий консервативный parser. Явный CLI adapter
предпочитает проверенную геометрию и не возвращается к bare prompt при её
неоднозначности: неизвестный экран остаётся целиком доступен для чтения.
Начальный display/hint не становится draft до точного подтверждения ввода
новым source frame. Hidden cursor, protected input, resize и смена источника
не дают нового доказательства. Смена adapter сбрасывает незавершённые proofs,
сохраняя уже накопленный архив ответов.

После принятого Backspace/Delete общее ядро может кратко удержать приватное
подтверждение исходного поля, если TUI разделил перерисовку на скрытый кадр и
хвост с показом каретки. Восстановление требует нового видимого source frame,
того же адаптера/источника/геометрии/палитры и точного удаления по UTF-16 с
наблюдаемой кареткой. Число клавиш не переводится в число Unicode-символов.
Первое нажатие задаёт предел 750 мс; повтор не продлевает его. Меню, прочие
клавиши, смена источника или protected input отменяют такое подтверждение.
Скрытый кадр не публикует старое или предсказанное содержимое поля.

Для Kimi/Gemini общий fallback также сохраняет исходное уже заполненное поле
Box/Rules: видимая каретка в конце непустого текста и полная текущая геометрия
подтверждают prefill без нового write. Это общий bounded-контракт, не Bare
fallback и не исключение из подтверждения hint. После unentered hint новый
текст по-прежнему требует exact source ACK; новая неизвестная подсказка при
каретке в начале не становится entered text.

| Adapter | Предпочтительная проекция | Дополнительная семантика |
| --- | --- | --- |
| Codex | Padded textarea, две колонки prefix, верхний/нижний padding; optional footer | Source-styled `Worked for` duration, paired bullet roles |
| Claude | Отдельные горизонтальные границы, pointer + separator, software cursor | Paired bullet roles; source-verified effort/elapsed chrome исключён из ответа |
| OpenCode | Gutter panel, textarea и отдельный toolbar | Ответ после принятого prompt panel, duration/error block этого ответа |
| Kimi | Однозначный box/rules с source acknowledgement | Paired plain `●` после принятого `❯` и source-styled Working над rounded composer |
| Gemini | Только тот же ограниченный bounded fallback | Generic labelled phases/status; renderer-specific completion пока выключен |

Generic completion/status остаётся доступным независимо от CLI. Текст примера
в code fence/quote и неподтверждённые сообщения не завершают ответ. Codex duration
не завершает Claude/Kimi/Gemini; OpenCode elapsed-summary не назначается чужому
adapter. Меню, trust/approval и auth экраны не подтверждаются автоматически.

## Повторная проверка preview.26

Все установленные CLI запускались с новыми private user/config/work folders,
без пользовательской авторизации. Собственные loopback HTTP/SSE providers
возвращали только фиксированный текст, без tool calls и внешней LLM-сети.
Это проверяет настоящий renderer и терминальную интеграцию, не качество модели.

| CLI | Версия | Два ответа и переходы по архиву | Выход |
| --- | --- | --- | --- |
| Codex | 0.164.0 | Два ID и Ready, previous/next; ввод только после готовности editor | `/quit`, code 0 |
| Claude | 2.1.240 | Два ID и Ready; effort/elapsed chrome не попадает в текст и речь ответа | `/exit`, code 0 |
| OpenCode | 2.0.32 | Два ID и Ready, previous/next по реальным drain/poll чанкам | standalone `/exit`, code 0 |
| Kimi | 2.4.1 | Два обычных ответа и повтор запроса во время stream; source ACK и Session replay | `/exit`; KeepTab code 0, OpenShell и новая команда |
| Gemini | Не установлен | Bounded fixtures passed; живой диалог skipped | Installed smoke skipped |

Codex/Claude/OpenCode записаны при 110×36, Kimi при 100×29. Canonical captures,
исходные принятые клавиши и временные метки polls сохранены в
`tests/fixtures/tui-cli-audit26/` и `tests/fixtures/tui-kimi-2.4.1-two-answers/`.
Пять Session-регрессий проходят общий путь source acknowledgement → composer,
history/menu → Submission → TurnLog → previous/next/режим ответа.

Отдельно воспроизведён возврат Kimi в Windows PowerShell 5 при длинном пути:
процесс жив, но fingerprint линейного приглашения меняется из-за ConPTY wrap.
Собственная runtime-пара OSC-маркеров исправляет handoff; PowerShell 5/7 и
первая новая команда проверены. Crash-recovery по ADR0018 не расширен;
подробности — ADR0021. Встроенные CLI-профили с preview.26 по умолчанию открывают
выбранную свежую оболочку после выхода; сохранённый KeepTab не меняется.

Endpoint-only replay не доказывает lifecycle: он пропускал промежуточные busy
controls и давал ложные первоначальные замечания к Codex/OpenCode. Принятые
проверки используют реальные чанки и тихие polls. Авторизованные сервисы,
физическая клавиатура, интерактивный desktop и речь NVDA требуют ручной приёмки.

## Codex

Проверены официальные [CLI reference](https://developers.openai.com/codex/cli/reference)
(перенаправляет в [Developer commands](https://learn.chatgpt.com/docs/developer-commands?surface=cli)).
Документированы `--no-alt-screen`, режимы TUI, запуск без начального prompt и
настройка footer/status line. Следовательно, наличие alternate screen или
одной фиксированной строки footer не является обязательным признаком CLI.
Эти настройки адаптер не изменяет.

Локально `codex-cli 0.164.0`, проверены `--version`/`--help`. Существующие
offline captures в `tests/fixtures/tui-codex-claude-composers/` подтверждают
normal-screen padded composer, hint, Unicode, Home/Right, удаление префикса,
clear и slash-меню. Для выбранного Codex допускается также footerless форма:
непрерывное textarea, ровно ограниченный padding и пустой хвост экрана.
Неизвестный непустой footer не поглощается этой веткой.

В preview.30 отдельный модуль `codex.rs` обрабатывает нативный Windows input
mode 9001. Модификаторы Enter передаются Win32 key records; BMP literal input
использует VK_PACKET, supplementary scalars остаются UTF-8. Реальная запись
подтверждает точную доставку 23 Unicode cases, включая комбинируемые знаки,
кириллицу, CJK, скобки, emoji и многострочный ввод. Shift+Enter/Alt+Enter
добавляют строку; CSI-u и Ctrl+J установленный Windows CLI игнорирует.

Только управляемый профиль подтверждённой версии получает
`-c tui.disable_paste_burst=true`. Старый шаблон обновляется лишь при точном
совпадении файла/версии и неизменённых настройках. Custom profiles и глобальный
config сохраняются. Полная вставка до 32768 UTF-16 подтверждается новым source
frame; большой paste имеет ограниченный срок 15 секунд, обычный ввод — 750 мс.
Нижнее поле содержит только фактически видимый текст. Для обрезанного поля
полный literal intent остаётся приватным; полный новый `›` echo после отправки
подтверждает весь запрос для архива. Переносы и repaint с ростом history
подтверждаются точным текстом и кареткой, а не фоновым WRAPLINE.

Во время ответа Tab регистрирует Queue, Enter — Steer. Новый request выбирается
по своему свежему echo, поэтому Steer может обогнать очередь, а одинаковые
запросы остаются разными. Исходный duration завершает свой ответ; исчезнувший
Working и постоянно доступный редактор Codex не дают Ready. Прерванный ответ
сохраняется отдельно. Стилевые preview очереди/steering исключены из ответа.
Настоящие записи и provenance: `tests/fixtures/tui-codex-production30/`.
Архитектурный контракт: [ADR0022](decisions/0022-codex-native-input-and-turns.md).

## Claude Code

Проверены [CLI reference](https://code.claude.com/docs/en/cli-reference) и
документированные `--ax-screen-reader`/`--bare`. Первый меняет renderer,
поэтому декоративные границы нельзя считать универсальным свойством всех
режимов. `--bare` уменьшает автоматическое обнаружение project integrations;
это свойство изолированных probes, не автоматически добавляемый флаг запуска.

Локально `2.1.240 (Claude Code)`, свежие help/version. Offline capture этого
бинарника подтверждает ruled input с pointer + NBSP, inverse cursor и Unicode
редактирование. Выбранная стратегия сохраняет hint как unentered display и
подтверждает вставку по source, а движение software cursor не меняет palette
пользовательского текста. Неподдержанный plain/AX layout остаётся читаемым
полным экраном; успешной AX/NVDA-приёмки это не означает.

## OpenCode

В preview.28 отдельный модуль `opencode.rs` задаёт его стратегию, кодирование
явных клавиш и параметры встроенного профиля. Обнаружение понимает реальный
вывод `opencode v2.0.32`. Только после подтверждения версии и `--standalone`
в ограниченном `--help` профиль добавляет этот аргумент: отдельный сервер
принадлежит запущенному клиенту, глобальная служба не перенастраивается.
При неизвестной версии/возможности сохраняется обычный запуск без добавленных
параметров. Пользовательские сохранённые варианты не переписываются.

Для встроенного профиля задаётся процессная настройка
`OPENCODE_CLI_CONFIG_CONTENT={"prompt":{"paste":"full"}}`, чтобы многострочная
вставка оставалась доступна исходным текстом вместо `[Pasted ~N lines]`.
OpenCode сам объединяет её с глобальным cli.json. Если пользователь уже задал
inline-переменную в родительском окружении, профиль сохраняет её приоритет.
Она также редактируется в собственной копии профиля. Никакие настройки
авторизации, провайдеров, разрешений или файлов пользователя не меняются.

Shift+Enter, Ctrl+Enter и Alt+Enter передаются с настоящими модификаторами
CSI-u; обычный Enter сохраняет CR. Это кодирование явного нажатия, решение
о действии остаётся у OpenCode и его keybinds. Первое полное bracketed paste
подтверждается до 32 768 символов только при выбранном OpenCode и известном
незащищённом исходном поле. Предел UTF-16 поля, геометрия, styles, точный текст,
каретка и срок 750 мс сохраняются. Word wrap renderer сверяется со всем
принятым текстом; мягкий экранный перенос не добавляется в отправленный запрос.
Неизменённый source сохраняет проверенное представление при движении каретки.
С preview.29 ввод сразу после первого появления формы поддерживается и до
завершения её начальной перерисовки: после появления полной геометрии
подтверждаются только точный префикс явно принятого текста, исходная позиция
в начале поля и видимая каретка. Без прежнего поля лимит остаётся 64 символа;
неизвестный готовый черновик из экрана не подхватывается по этому правилу.
При явном изменении размера прежнее подтверждение сохраняется частно до двух
секунд и переносится на новую геометрию только после нового исходного вывода,
при совпадении всего текста, режима, буфера и палитры. Любое новое принятое
нажатие отменяет это подтверждение, чтобы не потерять явную новую строку.
Backspace/Delete после wrapped-вставки подтверждают фактическое удаление по
исходным glyphs и позиции каретки. Количество UTF-16 единиц/символов заранее
не угадывается; неоднозначное удаление не даёт нового доказательства.

Стандартный Ctrl+X → Enter сохраняет кратковременное намерение поставить
подтверждённый запрос в очередь (до 3 секунд) и документ до leader-перерисовки.
Меню, защита, другой текст и другая клавиша отменяют это намерение. Разделение
ответов всё равно требует нового полного source echo; одна клавиша не делит
поток. Как в Kimi, при немедленном переходе к следующему ответу без стабильной
границы первый может остаться incomplete; оба тела и оба запроса доступны,
готовность последнего подтверждается отдельно. Режим Shell определяется только
в доказанном toolbar-слоте и не создаёт запросов к модели; его черновик доступен
в общем поле и после Escape. Команды из текста не запускаются.

Новые byte-exact captures и временные input/poll ledgers:
`crates/bks-app/tests/fixtures/tui-opencode-profile28/`. Настоящий установленный
2.0.32 проверен в отдельном ConPTY с private home/config/cwd: редактор без
HTTP-запросов, queued-запросы с двумя заранее заданными loopback-ответами,
естественный `/exit` code 0. Production Session replay проверяет полные запросы,
отдельные ID, источник Ready, previous/next и сворачивание запросов. Реальная
сеть модели, пользовательские сессии и ручная речь NVDA не использовались.

Проверены официальные [TUI](https://opencode.ai/v2/docs/cli/tui) и
[Settings](https://opencode.ai/v2/docs/cli/config). Они различают отправку,
вставку newline, steering/queue, shell mode и slash completion; настройки TUI
хранятся отдельно от server/project config. Adapter читает текущее состояние,
не подменяет эти действия локальными командами. Процессный режим полного
показа вставки — явное значение встроенного профиля, описанное выше.

Локально `opencode v2.0.32`, свежие help/version. Реальные изолированные
captures `tui-turns-opencode-*.vt` и меню подтверждают gutter composer, несколько
ответов, reflow/scroll и final error после retry. Ответ/summary принадлежат
принятому source prompt, а финальная ошибка сохраняется со стилями и завершает
turn с failed outcome. Эти соглашения включены именно в OpenCode policy.

## Kimi: два поколения

Прочитаны [legacy command reference](https://moonshotai.github.io/kimi-cli/en/reference/kimi-command.html),
[legacy keyboard reference](https://moonshotai.github.io/kimi-cli/en/reference/keyboard.html),
[официальная migration guide](https://moonshotai.github.io/kimi-code/en/guides/migration),
а также новый [Getting started](https://www.kimi.com/code/docs/en/kimi-code-cli/guides/getting-started)
и [Interaction and input](https://www.kimi.com/code/docs/en/kimi-code-cli/guides/interaction).
Migration guide прямо различает Python/uv и новое Node-поколение с другим TUI.
Новый интерфейс документирует input box, conversation, status bar, slash menu,
shell mode и отдельные approval panels. Старые сочетания нельзя переносить
в новый UI только по совпадению имени `kimi`.

Свежий локальный probe установил **2.4.1** в
`C:\Users\user\.kimi-code\bin\kimi.exe`: help содержит команды нового поколения
`provider`, `acp`, `migrate`, `upgrade`. Предыдущее предположение о Python 2.3.3
для этого пути не подтвердилось. Никакой установки, обновления или migration
в этой работе не выполнялось.

Первый собственный ConPTY 100×29, без auth и LLM, остановился на folder trust.
`tui-kimi-2.4.1-offline-trust.vt` сохраняет этот экран как negative fixture;
он не становится полем ввода и не подтверждается.

В preview.26 собственный пустой workspace получает только per-process
`KIMI_CODE_TRUST_WORKSPACE=1`; пользовательский trust/config не меняется.
Официальные [environment variables](https://www.kimi.com/code/docs/en/kimi-code-cli/configuration/env-vars.html)
и [providers](https://www.kimi.com/code/docs/en/kimi-code-cli/configuration/providers.html)
позволяют отдельный KIMI_CODE_HOME и OpenAI-compatible fixture на loopback.
Получены два настоящих ответа, затем повтор одинакового запроса во время
первого stream. Source ACK, архив/уникальные ID и предыдущий/следующий ответ
проверены по реальным чанкам. Обычные ответы завершаются после доказанного
исчезновения отдельно оформленной Working-строки над живым rounded composer.
При очереди без самостоятельной idle-границы первый ответ остаётся incomplete;
следующий завершается. Неизвестные controls остаются полным исходным текстом.
Captures и provenance — `tests/fixtures/tui-kimi-2.4.1-two-answers/`.
Это проверка настоящего renderer с фиксированными локальными ответами,
а не авторизованного сервиса, всех режимов или речи NVDA.

## Gemini CLI

Проверены официальные [docs](https://geminicli.com/docs/),
[keyboard shortcuts](https://geminicli.com/docs/reference/keyboard-shortcuts/),
[configuration](https://geminicli.com/docs/reference/configuration/) и
[commands](https://geminicli.com/docs/reference/commands/). Документированы
отдельные действия submit/newline/completion/dialog navigation, включаемый
alternate buffer и plain-text screen-reader mode. Поэтому adapter не выводит
состояние или разрешение из произвольного текста меню.

Локальный `gemini` не найден; он не устанавливался. Получена документационная
приёмка и тестирование generic bounded fallback/unknown-format preservation.
Нет real renderer capture, проверки версии, живой отправки или NVDA-приёмки.
Специфические признаки завершения/ошибок Gemini пока не включены.

## Артефакты и проверка

Свежие stdout `--help`/`--version` четырёх установленных CLI сохранены в
`target/profile-adapters/docs/<cli>/`. Процессы запускались с отдельными
HOME/USERPROFILE/APPDATA и config dirs, удалённым унаследованным окружением и
HTTP/HTTPS/ALL proxy на закрытый loopback port 9. Секреты, пользовательские
auth/config и существующие сессии не требовались.

Новая Kimi fixture: 1321 байт, полный startup собственного ConPTY, только
terminal protocol replies, без пользовательских writes; helper/manifest —
`target/profile-adapters/captures/kimi-2-4-1/`. Собственная PTY закрыта.
SHA256: `805F9E7CC7F220FCF02DD0B9CAE6A7FDDFE72C679C5E34D40D5F4909F28A78A3`.
Старые captures имеют provenance/endpoints в соседних README.

`tui-bounded-prefill-handoff.vt` — 4089 байт настоящего ConPTY 100×30 от того
же managed native fixture, что проверяет SingleInput GUI: shell history,
явный `kimi` + Enter и фиксированный box с Unicode prefill/caret end. Это
синтетический handoff без поиска настоящего CLI, LLM или пользовательских
настроек. Shell заканчивается на байте 3414; prefilled box — на 4089.
SHA256: `50820A625A1082540B9D0AB5210A9B94DAC8B587444892B9B26175FA5BA62B47`.
Helper, phase metadata и cleanup — `target/profile-adapters/bounded-fallback-capture/`.

`tui-padded-hidden-delete.vt` — 1679 байт точного producer output из неудачного
SingleInput GUI run `20261102-211938-e47d1d51`, original range 29660..31339.
Это только девять padded editor frames: hint → `Ж界🙂` → Home → Right →
удаление префикса → `界🙂` → End → удаление `🙂`.
SHA256: `24FF21C96B731D269B890B3636EA221AEF913CE0C6724BD7C27636125229581F`.
Контролируемый split 1673/1679 (original 31333/31339) отделяет изменённый
hidden frame от `ESC[?25h`. Glyph `界`, spacer и каретка row 2 / column 4
исправны в VT; регрессия проверяет потерю и guarded recovery ownership.
Это воспроизведение transport partition, а не утверждение о записанных
границах исходных ConPTY chunks. Fixture protocol и старые ожидания не менялись.

Адресные тесты запускаются с отдельным `--target-dir target/profile-adapters/verify`:
serde/default policies, optional footer, ruled hint/ACK, real Unicode и prefix
deletion, unknown/trust preservation, per-adapter semantics и смена policy без
потери архива. Общие workspace/release проверки выполняются при интеграции.
