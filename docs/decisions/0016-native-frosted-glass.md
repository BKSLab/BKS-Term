# ADR 0016 — матовое стекло в нативном интерфейсе

Дата: 2026-10-31. Статус: принято для 0.4.0-preview.13, уточнено в preview.14.

## Контекст

Референс требует стеклянной поверхности всего окна. Пользователь уточнил:
основа нейтральная серая, цвет приходит от окружения; расположение существующих
панелей сохраняется. Одна полупрозрачная шапка и сплошной фон вывода не подходят.
Win32, ConPTY и нативные доступные текстовые поверхности остаются основой.

## Решение

- `backdrop` выбирает материал с учётом High Contrast, настройки пользователя
  и запрета прозрачности Windows. Поддержку Windows 11 22H2+ проверяет через DWM.
- `accent_backdrop` изолирует динамический вызов Accent Acrylic, сохраняющий
  размытие без фокуса. Accent policy не является гарантированным контрактом DWM.
  При отказе используется системный Desktop Acrylic, затем непрозрачный фон.
- В режиме Accent приложение само рисует всю рамку: документированный
  `DWMNCRP_DISABLED` убирает сплошную системную заливку. `WM_NCPAINT` и UAH
  caption/frame messages подавляются. `WM_NCACTIVATE` неминимизированного
  окна возвращает TRUE без DefWindowProc: вариант с lParam=-1 на build 26200
  всё равно рисовал классическую рамку. Для WM_SETTEXT/WM_SETICON штатная
  обработка выполняется под кратким WM_SETREDRAW lock только видимого окна,
  затем инвалидируется client. Название, значки и исходная видимость сохраняются.
  `WS_OVERLAPPEDWINDOW`, размеры, hit testing и команды окна сохранены.
- `GlassBuffer` хранит premultiplied BGRA. Фон терминала плотнее шапки;
  текст, выделение и явные ANSI-фоны остаются непрозрачными. Текст оболочки
  и VT рисуется через `DrawThemeTextEx` с корректной alpha-композицией.
  В preview.14 ослаблены общий tint (128/255) и слой терминала (144/255):
  фон заметнее, а основное терминальное начертание на белом окружении
  сохраняет измеренный контраст 6,36:1 на build 26200.
- `terminal_glass` буферизует нативный RichEdit/EDIT через `WM_PRINTCLIENT`.
  RichEdit сохраняет HWND, документ, выделение, каретку и UIA provider;
  default background пропускается, а заданные ANSI-фоны рисуются нативно.
  Для EDIT прозрачным становится только его штатный фон в собственном DIB,
  без оконного color key и изменения hit testing.
- После операций, при которых RichEdit может рисовать вне `WM_PAINT`,
  запрашивается полная перерисовка. Native Freeze/WM_SETREDRAW сохраняются.
- При смене темы/шрифта инвалидируется кэш оформления RichEdit: Windows
  сбрасывает цвета, и совпадение старого кэша не доказывает сохранность стилей.

## Проверки и ограничения

Нативные тесты проверяют alpha фона и текста, selection/ANSI, очистку старого
содержимого, возврат обычных стилей и восстановление цветов после WM_SETFONT.
На Windows 11 build 26200 экранное A/B подтвердило размытие всего неактивного
окна и непрозрачный режим; scroll/selection не оставляют старых пикселей.
Геометрия, разворачивание и восстановление проверены относительно старой сборки.

NVDA, реальное масштабирование мониторов и удобство взаимодействия проверяет
человек. Наличие compatibility fallback не означает поддержку всех будущих
версий Windows. Исходные PNG и дизайн-спецификация не изменялись.

Источники API: [DWMNCRENDERINGPOLICY](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwmncrenderingpolicy),
[SetWindowCompositionAttribute](https://learn.microsoft.com/en-us/windows/win32/dwm/setwindowcompositionattribute),
[WM_PRINTCLIENT](https://learn.microsoft.com/en-us/windows/win32/gdi/wm-printclient),
[WM_NCACTIVATE](https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-ncactivate),
[WM_SETREDRAW](https://learn.microsoft.com/en-us/windows/win32/gdi/wm-setredraw).
