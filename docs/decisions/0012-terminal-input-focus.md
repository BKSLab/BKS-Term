# ADR0012: фокус живого TUI и чтение документа

Статус: принято, 2026-10-30. Уточняет ADR0010 и ADR0011.

Фокус и видимость элементов заменены в preview.10 решением
[ADR0014](0014-shared-source-owned-tui-input.md). Ниже описано поведение preview.8–9.

## Контекст

Ручная проверка preview.7 показала отдельные `/` и `)` при навигации Kimi
помимо подсказки выбранного пункта. Фокус находился в readonly RichEdit.
NVDA добавляет к нативному TextPattern обработку стрелок как навигации
каретки и читает текст после отправки клавиши даже без перемещения каретки.
Изменения только custom feedback не устраняют этот второй канал речи.

## Решение

Фокус живого ввода получает существующий BksTerminalSurface. Он принимает
те же клавиши через прежний message loop и ConPTY; не предоставляет
TextPattern редактора. Сохраняется одно место ввода в общей рабочей области.

Readonly RichEdit1011 остаётся видимым доступным companion за непрозрачным
canvas: текст, стиль и координаты доступны через штатный TextPattern,
но он исключён из Tab-порядка и перенаправляет raw focus на canvas.
Ctrl+Shift+Space и Shift+навигация открывают существующий VT reader1018.
В reader сохраняются обычная текстовая навигация, выделение и чтение формата;
Esc возвращает ввод. Цвета и clipboard/CRLF контракт ADR0011 сохраняются.

Canvas владеет TABSTOP и состоянием terminal_capture. Фокус/клавиши старого
CLI после возврата prompt проверяются до маршрутизации к командному редактору.
Прямой запуск known CLI регистрируется по принятым байтам ввода, а не
распознанному тексту TUI; неизвестное редактирование отменяет захват.

## Проверка

Keyless native harness проверяет реальный focus HWND canvas, отсутствие у
него TextPattern, сохранение полного readonly TextPattern companion,
z-order/clipping/геометрию и отсутствие лишних PTY-байтов. Человек отдельно
проверяет речь NVDA и работу реального Kimi; native проверки этого не заменяют.

Источник механизма NVDA:
[editableText.py](https://github.com/nvaccess/nvda/blob/master/source/editableText.py),
`_caretMovementScriptHelper` и `_caretScriptPostMovedHelper`;
[UIA overlays](https://github.com/nvaccess/nvda/blob/master/source/NVDAObjects/UIA/__init__.py),
`findOverlayClasses`.
