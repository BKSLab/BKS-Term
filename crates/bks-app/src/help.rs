use std::cell::{Cell, RefCell};
use std::ffi::c_void;

use crate::accessibility::set_accessible_name;
use crate::dpi;
use crate::theme::{draw_mode_button, Theme};
use windows::core::{w, Error, Result, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, FillRect, InvalidateRect, SetBkColor, SetTextColor, HDC,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{SetWindowTheme, DRAWITEMSTRUCT, EM_SETSEL, ODS_HOTLIGHT};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, IsWindowEnabled, SetFocus, VK_CONTROL, VK_ESCAPE, VK_F1, VK_MENU,
    VK_RETURN, VK_SHIFT, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetWindowRect,
    IsChild, IsDialogMessageW, IsIconic, IsWindow, IsWindowVisible, LoadCursorW, LoadIconW,
    MoveWindow, RegisterClassW, SendMessageW, SetForegroundWindow, SetWindowPos, ShowWindow,
    TranslateMessage, BN_CLICKED, BS_OWNERDRAW, ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY, HMENU,
    IDC_ARROW, MINMAXINFO, MSG, SWP_NOACTIVATE, SWP_NOZORDER, SW_RESTORE, SW_SHOW, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_CLOSE, WM_COMMAND, WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DPICHANGED,
    WM_DRAWITEM, WM_ERASEBKGND, WM_GETMINMAXINFO, WM_KEYDOWN, WM_NCDESTROY, WM_SETFOCUS,
    WM_SETFONT, WM_SETTINGCHANGE, WM_SIZE, WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN, WS_EX_CLIENTEDGE,
    WS_EX_CONTROLPARENT, WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

const ID_TEXT: usize = 1;
const ID_CLOSE: usize = 2;

const HELP_TEXT: &str = concat!(
    "BKS Term — справка\r\n\r\n",
    "Это терминал: введённые команды выполняет выбранная оболочка, например PowerShell или CMD. ",
    "BKS Term не выполняет команды и подтверждения, распознанные из текста приложений.\r\n\r\n",
    "ВКЛАДКИ И ПРОФИЛИ\r\n",
    "Ctrl+Shift+T и кнопка «+» открывают одно меню выбора профиля для новой вкладки. ",
    "Выберите профиль и нажмите Enter: вкладка создаётся после выбора. ",
    "Esc закрывает меню без создания вкладки.\r\n",
    "Ctrl+Shift+P или «Профили» слева — меню профилей, режима ввода и справки.\r\n",
    "Ctrl+Tab / Ctrl+Shift+Tab — следующая / предыдущая вкладка.\r\n",
    "Alt+1…9 — выбрать вкладку по её номеру.\r\n",
    "Ctrl+Shift+W — закрыть текущую вкладку; для работающего процесса требуется подтверждение.\r\n",
    "После завершения процесса вкладка и её вывод сохраняются, в состоянии указан код выхода. ",
    "Завершение или ошибка объявляется один раз. «Профили → Перезапустить в новой вкладке» ",
    "запускает свежую сессию того же профиля, оставляя прежний вывод доступным.\r\n\r\n",
    "ОКНА, ПАПКИ И НАСТРОЙКИ\r\n",
    "«Профили → Новое окно» или Ctrl+Shift+N сначала предлагает профиль и открывает отдельное окно. ",
    "«Новая вкладка в папке» или Ctrl+Shift+O открывает системный выбор папки, затем выбор профиля. ",
    "Отмена любого выбора не создаёт сессию. У каждой вкладки собственная начальная папка.\r\n",
    "В «Профили → Настройки» меняются озвучивание, сочетания и поведение последней вкладки. ",
    "Для сочетания выберите действие, введите клавиши, нажмите «Назначить», затем «Сохранить». ",
    "Алиасы разделяются точкой с запятой. F1 и Ctrl+Shift+Space обязательны для возврата управления. ",
    "Конфликтующие сочетания и клавиши обычного редактирования не назначаются глобальным действиям.\r\n",
    "Последняя закрытая вкладка по умолчанию оставляет пустое окно с кнопкой новой вкладки. ",
    "В настройках можно выбрать закрытие окна. Запоминание недавних профилей выключено по умолчанию; ",
    "сохраняются только имена профилей и начальные папки, без команд, черновиков и вывода. ",
    "Меню недавних профилей запускает только выбранный пользователем профиль. При обычном старте ",
    "BKS открывает локальную оболочку; сохранённые команды и SSH автоматически не восстанавливаются.\r\n",
    "Параметры запуска: --profile ID, --cwd ПАПКА, --choose-profile. ",
    "Интеграция Проводника включается явным запуском --register-explorer и отключается --unregister-explorer. ",
    "Она действует только для текущего пользователя; установка сама её не включает.\r\n\r\n",
    "ПАНЕЛЬ CODEX APP SERVER\r\n",
    "«Профили → Панель Codex App Server» открывает отдельный клиент совместимого Codex CLI 0.164.0. ",
    "Открытие окна не запускает сервер. «Запустить App Server» явно запускает локальный процесс; ",
    "«Отправить» передаёт введённое сообщение модели через установленный Codex и его существующий вход. ",
    "BKS сам не читает токены. Для озвучивания терминала внешняя модель не вызывается.\r\n",
    "Поддерживаются новый текстовый диалог, потоковый ответ, прерывание и разовое решение ",
    "для обычного запроса выполнения команды. Полный контекст решения доступен для чтения; ",
    "изменение запроса блокирует старый выбор. Стрелки и чтение не отправляют разрешение. ",
    "Для ответа требуется выбрать вариант и нажать отдельную кнопку передачи решения.\r\n",
    "Запросы входа, файлов, сетевых и постоянных разрешений остаются в обычном CLI. ",
    "«Открыть обычный терминал» создаёт новую вкладку в папке панели без повторной отправки текста. ",
    "Закрытие панели останавливает только её сервер и его дочерние процессы. ",
    "История панели ограничена и хранится в памяти.\r\n\r\n",
    "ОЗВУЧИВАНИЕ ВЫВОДА\r\n",
    "В меню «Профили» выберите один из трёх режимов. «Только важное» включён по умолчанию: ",
    "объявляются завершение и ошибки активной сессии. «Весь новый вывод» дополнительно читает ",
    "новые устойчивые строки текущего экрана, объединяя частые порции. ",
    "Повторная перерисовка, спиннеры и обновление строки прогресса не создают поток речи.\r\n",
    "«Чтение вручную» отключает автоматические сообщения вывода и завершения. ",
    "Ответы на ваши явные действия (отправка команды, поиск, выбор настройки) сохраняются. ",
    "Настройка сохраняется между запусками. Фоновые вкладки не озвучивают накопленный вывод при возврате. ",
    "Журнал и экран доступны для чтения вручную в любом режиме.\r\n\r\n",
    "ОБЗОР И ДИАЛОГИ TUI\r\n",
    "Shift+стрелки и Shift+Home/End выделяют текст от текущего курсора TUI в документе чтения. ",
    "Ctrl+Shift+Left/Right расширяют выделение по словам; Ctrl+C копирует текст. ",
    "Это выделение для чтения и копирования; Esc возвращает ввод, не отправляя Esc в приложение.\r\n",
    "Ctrl+Shift+D открывает обзор текущего экрана любого CLI: полный видимый текст и список строк. ",
    "В обзоре стрелки перемещают только чтение; Enter в списке ничего не отправляет. ",
    "Кнопка «Управлять клавишами TUI» явно включает живое управление в том же диалоге. ",
    "После этого стрелки, Tab, Enter и Esc передаются самому TUI по одному нажатию. ",
    "До ответа приложения следующая клавиша не ставится в очередь. Ctrl+Shift+I или «Читать снимок» возвращает к чтению. ",
    "Изменение исходного экрана перед нажатием блокирует старое действие; включите управление заново. ",
    "Возможное выделение — подсказка по оформлению. Кнопка «Вернуться к управлению TUI» ",
    "возвращает фокус в прямой ввод, где клавиши идут самому приложению. ",
    "При обновлении экрана исходный снимок сохраняется для чтения; откройте обзор заново для свежего текста.\r\n",
    "После явной навигации прямой ввод может объявлять перенос маркера, подсветки или инверсии, ",
    "а также переход видимого курсора на другую строку. Неоднозначные изменения не озвучиваются автоматически.\r\n",
    "При обнаружении совместимой установки в меню новой вкладки появляется «Codex CLI 0.164.0». ",
    "Адаптер работает только в отдельной вкладке этого профиля с проверенным исполняемым файлом. ",
    "Поддержан стартовый запрос Folder access: полный путь, предупреждение о доступе к файлам, ",
    "Trust and continue и Quit. Другие версии и экраны остаются обычным терминальным текстом.\r\n",
    "При распознавании появляется короткое уведомление без перемещения фокуса. ",
    "Ctrl+Shift+D или «Профили → Доступный диалог TUI» открывает нативное окно. ",
    "Сначала фокус в полном исходном тексте: его можно читать и копировать. ",
    "Tab переводит в список вариантов; стрелки меняют выбор без отправки. ",
    "Enter в списке или кнопка «Передать выбранный вариант в CLI» явно подтверждает выбранное действие.\r\n",
    "Esc и «Закрыть без отправки» закрывают только нативное окно. ",
    "При изменении экрана, вкладки или завершении процесса прежний выбор блокируется. ",
    "Откройте актуальный диалог заново. Ни распознавание, ни чтение, ни выбор стрелками ",
    "не посылают подтверждения. При недостаточно полном экране доступен обычный режим чтения.\r\n\r\n",
    "ПРОФИЛИ AI CLI\r\n",
    "Установленные Codex, Kimi, Claude и Gemini обнаруживаются в фоне и появляются в меню профилей. ",
    "Обычные профили используют собственный TUI инструмента и общий доступный буфер. ",
    "При подтверждённой поддержке отдельно предлагаются профили Claude и Gemini для экранного доступа. ",
    "Новая версия CLI сохраняет обычный запуск; неизвестная форма меню отключает только расширенные действия. ",
    "Если адаптер завершился или не ответил, чтение и ввод продолжают работать; повторить анализ можно явным Ctrl+Shift+D.\r\n\r\n",
    "SSH И ЗАЩИЩЁННЫЙ ВВОД\r\n",
    "«Профили → SSH-профили: создать или изменить» открывает нативный редактор. ",
    "Выберите новый или сохранённый SSH-профиль, задайте имя и адрес/алиас. ",
    "Пользователь, порт и путь к файлу ключа необязательны. Сохранение не подключается к серверу: ",
    "выберите сохранённый профиль при открытии вкладки. Изменение профиля не меняет уже запущенную сессию.\r\n",
    "Используется системный ssh.exe, включая обычную конфигурацию OpenSSH и ssh-agent. ",
    "Пароль, содержимое ключа и ключевая фраза в профиль не сохраняются. ",
    "Проверку host key выполняет OpenSSH; решение принимает пользователь.\r\n",
    "Когда программа ожидает скрытый пароль или ключевую фразу, выберите ",
    "«Профили → Защищённый ввод пароля или ключевой фразы». Поле скрывает символы. ",
    "Enter или «Отправить» передаёт значение и Enter в указанную сессию, обходя поле команды и историю. ",
    "Esc или «Отмена» очищает поле без отправки. Программа в терминале должна ожидать скрытый ввод; ",
    "диалог не определяет запрос по тексту и не отключает эхо дочерней программы.\r\n\r\n",
    "КОМАНДНЫЙ РЕЖИМ\r\n",
    "Обычная оболочка открывает нативное поле «Команда» и журнал чтения: выделение, удаление и Tab работают как в обычном редакторе. ",
    "Чтобы вернуться к ним из прямого режима, снимите отметку «Прямой ввод» в меню Ctrl+Shift+P.\r\n",
    "Введите текст в поле «Команда». Enter отправляет команду в оболочку, ",
    "Shift+Enter добавляет новую строку. Можно также нажать кнопку «Выполнить». ",
    "Пока оболочка выполняет команду, следующий текст остаётся в поле до появления prompt.\r\n",
    "После успешной отправки через Enter или «Выполнить» звучит короткое ",
    "«Команда отправлена»; фокус остаётся в поле команды. Это подтверждение отправки, ",
    "а результат выполнения читайте в выбранном документе.\r\n",
    "Tab / Shift+Tab перемещают фокус между элементами окна.\r\n\r\n",
    "ПРЯМОЙ ВВОД И ЧТЕНИЕ\r\n",
    "Пункт «Прямой ввод» в меню Ctrl+Shift+P переключает командный режим и VT-экран. ",
    "Отдельное поле команды скрывается. Этот режим нужен для интерактивных приложений и TUI.\r\n",
    "В отдельном CLI-профиле фокус находится в самой области TUI; дополнительного поля снизу нет. Обычные оболочки сохраняют командный редактор. ",
    "Простой запуск kimi, codex, claude или gemini из поля команды автоматически передаёт ввод TUI, без повторного запуска. ",
    "После возврата того же приглашения интегрированной оболочки восстанавливается поле команды. Во время чтения ответа позиция сохраняется до явного возврата к вводу. ",
    "Оно предоставляет NVDA живой VT-текст и каретку приложения; клавиши передаются в PTY, ",
    "а не редактируют документ. После явной навигации устойчивые изменения строк ",
    "кратко объявляются, включая видимый текст нового выбранного пункта. ",
    "Это чтение экрана, без автоматического выбора или подтверждения действий.\r\n",
    "Набор озвучивается только после подтверждённого появления текста на живом экране. ",
    "Скрытый и замаскированный ввод не становится эхом набора. Быстрые символы могут объединяться. ",
    "Ctrl+Shift+L читает текущую логическую строку, не отправляя клавиш в CLI. ",
    "Ctrl+Shift+R переносит к новому выводу после последнего Enter; при неизвестном начале открывается текущая страница.\r\n",
    "Ctrl+Shift+Space переводит фокус между вводом и выбранным источником чтения ",
    "и освобождает клавиатуру для чтения. Esc в документе или повторное сочетание возвращает к вводу с работающим Tab. В командном режиме поле команды и документ ",
    "остаются видимыми; в прямом режиме переключаются VT-экран и документ. ",
    "При включении прямого ввода источником чтения становится экран терминала. ",
    "Затем источник можно выбрать в меню; само сочетание Ctrl+Shift+Space его не меняет.\r\n",
    "Журнал получает текстовый поток в обоих режимах и предназначен для независимого чтения ",
    "и выделения текста, в том числе со скринридером. Это текстовый журнал, ",
    "он не воспроизводит расположение и обновления ячеек интерактивного VT-экрана.\r\n",
    "Журнал хранит до 512 КиБ текста или 6000 строк. Старые строки удаляются порциями. ",
    "Выделение и позиция чтения сохраняются в оставшемся тексте; удалённая позиция ",
    "переносится к его началу. Черновик команды сохраняется.\r\n",
    "В режиме TUI Tab / Shift+Tab получает CLI, включая автодополнение и навигацию. В обычной оболочке Tab обходит элементы окна. ",
    "Ctrl+Shift+I освобождает Tab для элементов окна; повторное сочетание или возвращение фокуса в терминал восстанавливает управление.\r\n",
    "При включённом захвате на VT-экране обычные клавиши, стрелки, Home/End, Insert/Delete, ",
    "PageUp/PageDown, F2…F12, Enter, Esc и Tab передаются приложению. ",
    "Ctrl+C тоже передаётся приложению: обычно это прерывание команды. ",
    "Esc не закрывает главное окно.\r\n",
    "Для возврата к навигации Tab снимите захват через Ctrl+Shift+I. ",
    "Переход к чтению через Ctrl+Shift+Space или открытие справки через F1 также снимает захват. ",
    "Alt+Tab переключает программы Windows.\r\n",
    "Ctrl+Shift+V или Shift+Insert — вставить текст из буфера обмена в прямом режиме. ",
    "При поддержке приложением используется bracketed paste. Вставка может содержать переводы строк. ",
    "При захвате Ctrl+V передаётся приложению как Ctrl+V.\r\n",
    "Shift+PageUp / Shift+PageDown — прокрутить историю VT-экрана.\r\n",
    "Ctrl+Shift+End — вернуться к последним строкам VT-экрана.\r\n\r\n",
    "Выделите текст на VT-экране мышью; Ctrl+Shift+C скопирует выделенное. ",
    "Без выделения Ctrl+Shift+C переходит к последнему доступному блоку результата в журнале ",
    "и копирует его.\r\n\r\n",
    "ЧТЕНИЕ ЭКРАНА И ЖУРНАЛА\r\n",
    "В меню Ctrl+Shift+P выберите «Читать экран терминала» или «Читать журнал команд». ",
    "Это два отдельных нативных текстовых источника в том же окне.\r\n",
    "Поле «Экран терминала» доступно только для чтения и отражает текущее содержимое VT: ",
    "основной экран вместе с историей, с учётом перезаписи текста. ",
    "При переходе приложения в альтернативный экран читается только он; ",
    "после выхода снова доступны основной экран и его история.\r\n",
    "Переносы длинной строки по ширине экрана объединены; явные переводы строк сохранены. ",
    "Чтение документа не меняет прокрутку графического VT-экрана. ",
    "Текст можно выделять и копировать обычным Ctrl+C.\r\n",
    "При новом выводе и изменении размера окна сохраняются выделение и логическое место чтения. ",
    "Основной и альтернативный экраны сохраняют свои позиции отдельно. ",
    "Если читаемый фрагмент удалён из истории, выбирается ближайшая сохранившаяся строка.\r\n",
    "Автопрокрутка за новым выводом работает только без выделения, ",
    "когда каретка стоит в конце документа и окно чтения прокручено вниз. ",
    "При чтении старых строк новый вывод не переносит вас в конец.\r\n",
    "«Читать журнал команд» возвращает текстовый поток и навигацию по блокам. ",
    "Ctrl+Up / Ctrl+Down в TUI переходят к чтению соседних логических строк без передачи клавиш приложению. В обычной оболочке они сохраняют навигацию по блокам журнала.\r\n\r\n",
    "РЕЗУЛЬТАТЫ КОМАНД В ЖУРНАЛЕ\r\n",
    "Блоки создаются для команд, отправленных через поле «Команда». ",
    "Если оболочка не сообщает границы результата, используется граница отправки команды. ",
    "Enter в прямом режиме не создаёт такой блок. Для навигации по блокам используйте ",
    "командный режим: снимите отметку «Прямой ввод» в меню Ctrl+Shift+P.\r\n",
    "Ctrl+Up / Ctrl+Down — предыдущий / следующий блок результата. ",
    "Из поля команды Ctrl+Up переходит к последнему блоку, Ctrl+Down — к первому. ",
    "На границе истории переход остаётся в крайнем блоке.\r\n",
    "Alt+Up / Alt+Down — предыдущий / следующий результат команды.\r\n",
    "Ctrl+Shift+C — скопировать результат текущего блока.\r\n",
    "В журнале доступны стандартные выделение текста и Ctrl+C для копирования выделенного.\r\n\r\n",
    "ПОИСК ПО ТЕРМИНАЛУ\r\n",
    "Ctrl+F открывает строку поиска и переводит фокус в запрос. ",
    "Поиск ищет буквальный текст без учёта регистра в документе VT, включая сохранённую историю. ",
    "В альтернативном экране поиск ограничен этим экраном.\r\n",
    "При открытии из команды или журнала показывается документ «Экран терминала». ",
    "При открытии из графического VT он остаётся видимым с подсветкой совпадений.\r\n",
    "Пока поиск открыт, F3 или «Далее» выбирают следующее совпадение, ",
    "Shift+F3 или «Назад» — предыдущее. Переход циклический. Счётчик показывает «1 из N»; ",
    "при переходе через край — «1 из N — с начала» или «N из N — с конца». ",
    "Пустой запрос показывает «Введите текст», отсутствие совпадений — «Ничего не найдено».\r\n",
    "После короткой паузы во вводе запроса звучит короткий актуальный статус. ",
    "F3, «Назад», «Далее» и Enter объявляют статус сразу после действия. ",
    "Ввод запроса и F3 сохраняют фокус; приложение не начинает читать весь вывод, ",
    "команды или приглашения оболочки. Фоновый вывод сам по себе не повторяет статус.\r\n",
    "Enter в запросе переводит фокус на уже выбранное совпадение в нативном документе VT, ",
    "где текст можно прочитать и скопировать. Для следующего совпадения используйте F3 ",
    "или «Далее».\r\n",
    "Esc или «Закрыть» скрывают строку поиска и возвращают прежний источник и элемент фокуса. ",
    "Если поиск был открыт из документа VT, выбранный результат остаётся местом чтения. ",
    "Запрос, выбранное совпадение и открытое состояние поиска сохраняются отдельно для каждой вкладки.\r\n\r\n",
    "СПРАВКА И ОГРАНИЧЕНИЯ\r\n",
    "F1 открывает эту справку или активирует уже открытое окно. ",
    "Текст можно выделять и копировать. Tab / Shift+Tab переключают текст и кнопку «Закрыть». ",
    "Esc закрывает только справку и возвращает фокус в главное окно.\r\n",
);

#[derive(Clone, Copy)]
struct HelpWindow {
    hwnd: HWND,
    text: HWND,
    close: HWND,
    owner: HWND,
    previous_focus: HWND,
    dpi: u32,
    theme: Theme,
}

thread_local! {
    static HELP: RefCell<Option<HelpWindow>> = const { RefCell::new(None) };
    static CLASS_REGISTERED: Cell<bool> = const { Cell::new(false) };
}

fn current() -> Option<HelpWindow> {
    HELP.with(|help| *help.borrow())
}

pub fn is_open() -> bool {
    current().is_some_and(|help| unsafe { IsWindow(Some(help.hwnd)).as_bool() })
}

/// Show a single modeless help window on the application's UI thread.
pub fn show(owner: HWND) -> Result<()> {
    if is_open() {
        activate(current().expect("open help has a window"));
        return Ok(());
    }
    unsafe {
        let instance = HINSTANCE(GetModuleHandleW(None)?.0);
        register_class(instance)?;
        let dpi = GetDpiForWindow(owner).max(dpi::BASE_DPI);
        let width = dpi::scale(820, dpi);
        let height = dpi::scale(640, dpi);
        let mut owner_rect = RECT::default();
        GetWindowRect(owner, &mut owner_rect)?;
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            w!("BksTermHelpWindow"),
            w!("Справка — BKS Term"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            owner_rect.left + (owner_rect.right - owner_rect.left - width) / 2,
            owner_rect.top + (owner_rect.bottom - owner_rect.top - height) / 2,
            width,
            height,
            Some(owner),
            None,
            Some(instance),
            None,
        )?;
        let help = HelpWindow {
            hwnd,
            text: HWND::default(),
            close: HWND::default(),
            owner,
            previous_focus: GetFocus(),
            dpi,
            theme: Theme::new(dpi),
        };
        HELP.with(|state| *state.borrow_mut() = Some(help));
        if let Err(error) = create_controls(help, instance) {
            let _ = DestroyWindow(hwnd);
            return Err(error);
        }
        let help = current().expect("help was initialized");
        apply_theme(help);
        layout(help);
        activate(help);
    }
    Ok(())
}

/// Run this before the main window's shortcuts and dialog navigation.
/// Messages belonging to help are dispatched here so its keys stay in help.
pub fn handle_message(message: &MSG) -> bool {
    let Some(help) = current() else {
        return false;
    };
    unsafe {
        if message.hwnd != help.hwnd && !IsChild(help.hwnd, message.hwnd).as_bool() {
            return false;
        }
        if message.message == WM_KEYDOWN {
            if message.wParam.0 == VK_TAB.0 as usize
                && GetKeyState(VK_CONTROL.0 as i32) >= 0
                && GetKeyState(VK_MENU.0 as i32) >= 0
            {
                let backwards = GetKeyState(VK_SHIFT.0 as i32) < 0;
                if let Some(next) = crate::keyboard_navigation::next_tab_stop(
                    &[help.text, help.close],
                    GetFocus(),
                    backwards,
                ) {
                    let _ = SetFocus(Some(next));
                }
                return true;
            }
            if message.wParam.0 == 0x41
                && GetFocus() == help.text
                && GetKeyState(VK_CONTROL.0 as i32) < 0
                && GetKeyState(VK_MENU.0 as i32) >= 0
            {
                SendMessageW(help.text, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
                return true;
            }
            if message.wParam.0 == VK_ESCAPE.0 as usize
                || (message.wParam.0 == VK_RETURN.0 as usize && GetFocus() == help.close)
            {
                close(help);
                return true;
            }
            if message.wParam.0 == VK_F1.0 as usize {
                activate(help);
                return true;
            }
        }
        if !IsDialogMessageW(help.hwnd, message).as_bool() {
            let _ = TranslateMessage(message);
            DispatchMessageW(message);
        }
    }
    true
}

unsafe fn register_class(instance: HINSTANCE) -> Result<()> {
    if CLASS_REGISTERED.get() {
        return Ok(());
    }
    let class = WNDCLASSW {
        hInstance: instance,
        hCursor: LoadCursorW(None, IDC_ARROW)?,
        hIcon: LoadIconW(Some(instance), w!("BKS_TERM")).unwrap_or_default(),
        lpszClassName: w!("BksTermHelpWindow"),
        lpfnWndProc: Some(window_proc),
        ..Default::default()
    };
    if RegisterClassW(&class) == 0 {
        return Err(Error::from_thread());
    }
    CLASS_REGISTERED.set(true);
    Ok(())
}

unsafe fn create_controls(help: HelpWindow, instance: HINSTANCE) -> Result<()> {
    let content = format!(
        "BKS Term {}\r\n\r\n{}{}{}",
        env!("CARGO_PKG_VERSION"),
        crate::shortcut_help_text(),
        crate::cli_help_text(),
        HELP_TEXT
    );
    let text: Vec<u16> = content.encode_utf16().chain(Some(0)).collect();
    // Standard UIA proxies use the preceding native STATIC as the EDIT name.
    let _label = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("STATIC"),
        w!("Справка BKS Term, текст, только чтение"),
        WS_CHILD,
        0,
        0,
        0,
        0,
        Some(help.hwnd),
        None,
        Some(instance),
        None,
    )?;
    let edit = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        w!("EDIT"),
        PCWSTR(text.as_ptr()),
        WS_CHILD
            | WS_VISIBLE
            | WS_TABSTOP
            | WS_VSCROLL
            | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY) as u32),
        0,
        0,
        0,
        0,
        Some(help.hwnd),
        Some(HMENU(ID_TEXT as *mut c_void)),
        Some(instance),
        None,
    )?;
    let button = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("BUTTON"),
        w!("Закрыть"),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
        0,
        0,
        0,
        0,
        Some(help.hwnd),
        Some(HMENU(ID_CLOSE as *mut c_void)),
        Some(instance),
        None,
    )?;
    HELP.with(|state| {
        if let Some(help) = state.borrow_mut().as_mut() {
            help.text = edit;
            help.close = button;
        }
    });
    crate::control_state::install(button);
    set_accessible_name(edit, "Справка BKS Term, текст, только чтение")?;
    set_accessible_name(button, "Закрыть справку")?;
    SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(0)));
    Ok(())
}

fn activate(help: HelpWindow) {
    unsafe {
        let _ = ShowWindow(
            help.hwnd,
            if IsIconic(help.hwnd).as_bool() {
                SW_RESTORE
            } else {
                SW_SHOW
            },
        );
        let _ = SetForegroundWindow(help.hwnd);
        let _ = SetFocus(Some(help.text));
    }
}

fn close(help: HelpWindow) {
    unsafe {
        let _ = DestroyWindow(help.hwnd);
        if IsWindow(Some(help.owner)).as_bool() {
            let _ = SetForegroundWindow(help.owner);
            let focus = if IsWindow(Some(help.previous_focus)).as_bool()
                && IsWindowVisible(help.previous_focus).as_bool()
                && IsWindowEnabled(help.previous_focus).as_bool()
                && (help.previous_focus == help.owner
                    || IsChild(help.owner, help.previous_focus).as_bool())
            {
                help.previous_focus
            } else {
                help.owner
            };
            let _ = SetFocus(Some(focus));
        }
    }
}

fn apply_theme(help: HelpWindow) {
    unsafe {
        for control in [help.text, help.close] {
            SendMessageW(
                control,
                WM_SETFONT,
                Some(WPARAM(help.theme.ui_font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
        let theme_name = if help.theme.high_contrast {
            PCWSTR::null()
        } else {
            w!("DarkMode_Explorer")
        };
        let _ = SetWindowTheme(help.text, theme_name, PCWSTR::null());
        let dark = u32::from(!help.theme.high_contrast);
        let _ = DwmSetWindowAttribute(
            help.hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
    }
}

fn layout(help: HelpWindow) {
    unsafe {
        let mut client = RECT::default();
        if GetClientRect(help.hwnd, &mut client).is_err() {
            return;
        }
        let margin = dpi::scale(20, help.dpi);
        let gap = dpi::scale(14, help.dpi);
        let button_width = dpi::scale(120, help.dpi);
        let button_height = dpi::scale(36, help.dpi);
        let bottom = (client.bottom - margin - button_height).max(margin);
        let _ = MoveWindow(
            help.text,
            margin,
            margin,
            (client.right - 2 * margin).max(1),
            (bottom - gap - margin).max(1),
            true,
        );
        let _ = MoveWindow(
            help.close,
            (client.right - margin - button_width).max(margin),
            bottom,
            button_width,
            button_height,
            true,
        );
    }
}

fn refresh_theme(help: HelpWindow, dpi: u32) {
    let next = HelpWindow {
        dpi,
        theme: Theme::new(dpi),
        ..help
    };
    HELP.with(|state| *state.borrow_mut() = Some(next));
    apply_theme(next);
    layout(next);
    release_theme(help.theme);
    unsafe {
        let _ = InvalidateRect(Some(help.hwnd), None, true);
    }
}

fn release_theme(theme: Theme) {
    unsafe {
        for font in [theme.ui_font, theme.title_font, theme.terminal_font] {
            if !font.0.is_null() {
                let _ = DeleteObject(font.into());
            }
        }
        if !theme.high_contrast {
            for brush in [
                theme.window,
                theme.terminal,
                theme.toolbar,
                theme.sidebar,
                theme.surface,
                theme.hover,
                theme.selected,
                theme.border,
                theme.border_subtle,
                theme.brand,
            ] {
                if !brush.0.is_null() {
                    let _ = DeleteObject(brush.into());
                }
            }
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let Some(help) = current().filter(|help| help.hwnd == hwnd) else {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    };
    match message {
        WM_SETFOCUS => {
            let _ = SetFocus(Some(help.text));
            LRESULT(0)
        }
        WM_CLOSE => {
            close(help);
            LRESULT(0)
        }
        WM_COMMAND if wparam.0 & 0xffff == ID_CLOSE && (wparam.0 >> 16) as u32 == BN_CLICKED => {
            close(help);
            LRESULT(0)
        }
        WM_SIZE => {
            layout(help);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            refresh_theme(help, ((wparam.0 & 0xffff) as u32).max(dpi::BASE_DPI));
            let rect = &*(lparam.0 as *const RECT);
            let _ = SetWindowPos(
                hwnd,
                None,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            refresh_theme(help, help.dpi);
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            if lparam.0 != 0 {
                let info = &mut *(lparam.0 as *mut MINMAXINFO);
                info.ptMinTrackSize = POINT {
                    x: dpi::scale(420, help.dpi),
                    y: dpi::scale(300, help.dpi),
                };
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => {
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            FillRect(HDC(wparam.0 as *mut c_void), &client, help.theme.window);
            LRESULT(1)
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            let hdc = HDC(wparam.0 as *mut c_void);
            SetTextColor(hdc, help.theme.colors.text_primary);
            SetBkColor(hdc, help.theme.colors.surface);
            LRESULT(help.theme.surface.0 as isize)
        }
        WM_DRAWITEM if wparam.0 == ID_CLOSE && lparam.0 != 0 => {
            let mut item = *(lparam.0 as *const DRAWITEMSTRUCT);
            if crate::control_state::is_hot(item.hwndItem) {
                item.itemState.0 |= ODS_HOTLIGHT.0;
            }
            draw_mode_button(&item, &help.theme, "Закрыть");
            LRESULT(1)
        }
        WM_NCDESTROY => {
            HELP.with(|state| *state.borrow_mut() = None);
            release_theme(help.theme);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}
