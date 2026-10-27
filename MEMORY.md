# BKS Term — память проекта (рабочее состояние)

**Обновлено:** 27 октября 2026 г.
**Назначение:** снимок текущего состояния для продолжения работы в новых сессиях агента. Обновлять после каждой завершённой итерации. Концепция и план — в `docs/concept.md` и `docs/setup-and-iterations.md`; при противоречии те документы важнее.

## 1. Где мы находимся

### Последний результат — 27.10.2026, 0.4.0-preview.6

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
