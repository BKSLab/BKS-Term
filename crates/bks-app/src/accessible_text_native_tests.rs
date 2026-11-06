//! Native regressions on private windows. No activation, keyboard injection or
//! clipboard write is used; UIA runs on a separate MTA while the owner pumps.
use super::*;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use bks_terminal::{Color, Flags, Rgb, TextStyle};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Variant::{VariantClear, VariantToInt32};
use windows::Win32::UI::Accessibility::{
    CUIAutomation8, IUIAutomation2, IUIAutomationTextPattern, UIA_BackgroundColorAttributeId,
    UIA_ForegroundColorAttributeId, UIA_TextPatternId,
};
use windows::Win32::UI::Controls::RichEdit::{tomFalse, tomTrue};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, DispatchMessageW, GetWindowLongW, GetWindowTextLengthW,
    IsWindowVisible, PeekMessageW, TranslateMessage, ES_MULTILINE, ES_READONLY, GWL_STYLE, MSG,
    PM_REMOVE, WINDOW_EX_STYLE, WINDOW_STYLE, WS_CHILD, WS_EX_NOACTIVATE, WS_VISIBLE, WS_VSCROLL,
};

struct HiddenRichEdit {
    parent: HWND,
    edit: HWND,
    _apartment: crate::accessibility::ComApartment,
    _native_fixture: std::sync::MutexGuard<'static, ()>,
}

impl HiddenRichEdit {
    fn new(source: &str) -> Self {
        let native_fixture = NATIVE_TEXT_FIXTURE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let apartment = crate::accessibility::ComApartment::initialize().unwrap();
        load().unwrap();
        let parent = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE,
                w!("STATIC"),
                w!("BKS private rich text fixture"),
                WINDOW_STYLE::default(),
                0,
                0,
                640,
                240,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        };
        let mut fixture = Self {
            parent,
            edit: HWND::default(),
            _apartment: apartment,
            _native_fixture: native_fixture,
        };
        fixture.edit = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("RICHEDIT50W"),
                w!(""),
                WS_CHILD
                    | WS_VISIBLE
                    | WS_VSCROLL
                    | WINDOW_STYLE((ES_MULTILINE | ES_READONLY) as u32),
                0,
                0,
                620,
                220,
                Some(parent),
                None,
                None,
                None,
            )
            .unwrap()
        };
        register(fixture.edit, "").unwrap();
        set_text(fixture.edit, source);
        assert!(!unsafe { IsWindowVisible(fixture.parent).as_bool() });
        fixture
    }

    fn document(&self) -> ITextDocument {
        CONTROLS.with(|controls| controls.borrow()[&(self.edit.0 as usize)].document.clone())
    }

    fn native_text(&self, start: u32, end: u32) -> String {
        unsafe {
            self.document()
                .Range(
                    native_offset(self.edit, start) as i32,
                    native_offset(self.edit, end) as i32,
                )
                .unwrap()
                .GetText()
                .unwrap()
                .to_string()
                .replace('\r', "\r\n")
        }
    }

    fn copied_model_text(&self) -> String {
        // Exactly the source/range consumed by WM_COPY, without invoking
        // WM_COPY or changing the user's clipboard.
        let (start, end) = selection(self.edit);
        let source = text(self.edit).unwrap();
        let units: Vec<_> = source.encode_utf16().collect();
        String::from_utf16(&units[start as usize..end as usize]).unwrap()
    }

    fn font(&self, start: u32, end: u32) -> windows::Win32::UI::Controls::RichEdit::ITextFont {
        unsafe {
            self.document()
                .Range(
                    native_offset(self.edit, start) as i32,
                    native_offset(self.edit, end) as i32,
                )
                .unwrap()
                .GetFont()
                .unwrap()
        }
    }
}

impl Drop for HiddenRichEdit {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.parent);
        }
    }
}

fn style(foreground: (u8, u8, u8), background: (u8, u8, u8), flags: Flags) -> TextStyle {
    let color = |(r, g, b)| Color::Spec(Rgb { r, g, b });
    TextStyle::new(color(foreground), color(background), flags)
}

fn span(start: u32, end: u32, style: TextStyle) -> StyleSpan {
    StyleSpan { start, end, style }
}

#[test]
fn native_unicode_crlf_selection_replace_and_destroy_preserve_the_model() {
    let source = "Ж🙂\r\ne\u{301}\r\nlast";
    let fixture = HiddenRichEdit::new(source);
    let hwnd = fixture.edit;
    assert_eq!(length(hwnd), Some(13));
    assert_eq!(text(hwnd).as_deref(), Some(source));
    assert_eq!(fixture.native_text(0, 13), source);
    for (start, end, expected) in [(0, 3, "Ж🙂"), (3, 5, "\r\n"), (5, 7, "e\u{301}")] {
        select(hwnd, start, end);
        assert_eq!(selection(hwnd), (start, end));
        assert_eq!(fixture.copied_model_text(), expected);
        assert_eq!(fixture.native_text(start, end), expected);
    }
    let next = "Ж🙂\r\n界🙂\r\nX\r\nlast";
    replace(hwnd, 5, 7, "界🙂\r\nX", next);
    assert_eq!(length(hwnd), Some(17));
    assert_eq!(selection(hwnd), (11, 11));
    assert_eq!(fixture.native_text(0, 17), next);
    select(hwnd, 5, 11);
    assert_eq!(fixture.copied_model_text(), "界🙂\r\nX");
    drop(fixture);
    assert!(!CONTROLS.with(|controls| controls.borrow().contains_key(&(hwnd.0 as usize))));
}

#[test]
fn native_style_only_changes_preserve_selection_and_skip_unchanged_formatting() {
    let fixture = HiddenRichEdit::new("A\r\nЖ🙂e\u{301}Z");
    let hwnd = fixture.edit;
    let theme = Theme::default();
    let plain = TextStyle::default();
    let bright = style(
        (0x12, 0x34, 0x56),
        (1, 2, 3),
        Flags::BOLD | Flags::UNDERLINE,
    );
    let first = vec![span(0, 3, plain), span(3, 8, bright), span(8, 9, plain)];
    select(hwnd, 3, 8);
    assert_eq!(format(hwnd, &first, &theme).unwrap(), 3);
    assert_eq!(selection(hwnd), (3, 8));
    assert_ne!(unsafe { GetWindowLongW(hwnd, GWL_STYLE) } & ES_READONLY, 0);
    let font = fixture.font(3, 8);
    unsafe {
        assert_eq!(font.GetForeColor().unwrap(), 0x56_34_12);
        assert_eq!(font.GetBackColor().unwrap(), 0x03_02_01);
        assert_eq!(font.GetBold().unwrap(), tomTrue.0);
        assert_ne!(font.GetUnderline().unwrap(), 0);
    }
    assert_eq!(format(hwnd, &first, &theme).unwrap(), 0);
    let changed = style((0xAA, 0xBB, 0xCC), (4, 5, 6), Flags::ITALIC);
    let next = vec![span(0, 3, plain), span(3, 8, changed), span(8, 9, plain)];
    assert_eq!(format(hwnd, &next, &theme).unwrap(), 1);
    assert_eq!(selection(hwnd), (3, 8));
    assert_ne!(unsafe { GetWindowLongW(hwnd, GWL_STYLE) } & ES_READONLY, 0);
    assert_eq!(fixture.copied_model_text(), "Ж🙂e\u{301}");
    unsafe {
        let font = fixture.font(3, 8);
        assert_eq!(font.GetForeColor().unwrap(), 0xCC_BB_AA);
        assert_eq!(font.GetBackColor().unwrap(), 0x06_05_04);
        assert_eq!(font.GetBold().unwrap(), tomFalse.0);
        assert_eq!(font.GetItalic().unwrap(), tomTrue.0);
    }
    assert_eq!(format(hwnd, &next, &theme).unwrap(), 0);
}

#[test]
fn native_appearance_refresh_restores_colors_and_preserves_text_selection_and_offsets() {
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{GetStockObject, DEFAULT_GUI_FONT};
    use windows::Win32::UI::Controls::SetWindowTheme;
    use windows::Win32::UI::WindowsAndMessaging::WM_SETFONT;

    for (refresh_theme, refresh_font) in [(true, true), (false, true), (true, false)] {
        let source = "A\r\nЖ🙂e\u{301}Z";
        let fixture = HiddenRichEdit::new(source);
        let hwnd = fixture.edit;
        let mut theme = Theme::default();
        theme.colors.terminal = COLORREF(0x25_10_0A);
        let styles = [
            span(0, 3, TextStyle::default()),
            span(3, 8, style((0x12, 0x34, 0x56), (1, 2, 3), Flags::BOLD)),
            span(8, 9, TextStyle::default()),
        ];
        select(hwnd, 3, 8);
        assert_eq!(format(hwnd, &styles, &theme).unwrap(), 3);
        assert_eq!(format(hwnd, &styles, &theme).unwrap(), 0);
        let offsets: Vec<_> = (0..=9)
            .map(|offset| (native_offset(hwnd, offset), canonical_offset(hwnd, offset)))
            .collect();

        unsafe {
            // apply_fonts() runs this sequence after saving preferences. A
            // Either native message must also invalidate formatting on its own.
            if refresh_theme {
                SetWindowTheme(hwnd, w!("DarkMode_Explorer"), PCWSTR::null()).unwrap();
            }
            if refresh_font {
                SendMessageW(
                    hwnd,
                    WM_SETFONT,
                    Some(WPARAM(GetStockObject(DEFAULT_GUI_FONT).0 as usize)),
                    Some(LPARAM(1)),
                );
            }
        }
        let reapplied = format(hwnd, &styles, &theme).unwrap();
        for span in &styles {
            let expected = resolve(span.style, &theme);
            let font = fixture.font(span.start, span.end);
            unsafe {
                assert_eq!(
                    font.GetForeColor().unwrap(),
                    expected.foreground.0 as i32,
                    "foreground after refresh (theme={refresh_theme}, font={refresh_font})"
                );
                assert_eq!(
                    font.GetBackColor().unwrap(),
                    expected.background.0 as i32,
                    "background after refresh (theme={refresh_theme}, font={refresh_font})"
                );
            }
        }
        assert_eq!(selection(hwnd), (3, 8));
        assert_eq!(text(hwnd).as_deref(), Some(source));
        assert_eq!(length(hwnd), Some(9));
        assert_eq!(fixture.native_text(0, 9), source);
        assert_eq!(fixture.copied_model_text(), "Ж🙂e\u{301}");
        assert_eq!(
            (0..=9)
                .map(|offset| (native_offset(hwnd, offset), canonical_offset(hwnd, offset)))
                .collect::<Vec<_>>(),
            offsets
        );
        assert_eq!(reapplied, styles.len());
        assert_eq!(format(hwnd, &styles, &theme).unwrap(), 0);
    }
}

#[test]
fn native_crlf_split_between_chunks_remains_one_hard_line_break() {
    let fixture = HiddenRichEdit::new("a\r");
    let hwnd = fixture.edit;
    assert_eq!(fixture.native_text(0, 2), "a\r\n");
    replace(hwnd, 2, 2, "\nb", "a\r\nb");
    assert_eq!(length(hwnd), Some(4));
    assert_eq!(text(hwnd).as_deref(), Some("a\r\nb"));
    assert_eq!(fixture.native_text(0, 4), "a\r\nb");
    select(hwnd, 3, 4);
    assert_eq!(fixture.native_text(3, 4), "b");
    assert_eq!(fixture.copied_model_text(), "b");

    set_text(hwnd, "Ж🙂\r");
    append(hwnd, "\nnext\r");
    append(hwnd, "\nlast");
    assert_eq!(text(hwnd).as_deref(), Some("Ж🙂\r\nnext\r\nlast"));
    assert_eq!(
        fixture.native_text(0, length(hwnd).unwrap()),
        "Ж🙂\r\nnext\r\nlast"
    );
    select(hwnd, 11, 15);
    assert_eq!(fixture.copied_model_text(), "last");

    set_text(hwnd, "a\rx\nb");
    replace(hwnd, 2, 3, "", "a\r\nb");
    assert_eq!(fixture.native_text(0, 4), "a\r\nb");
    assert_eq!(text(hwnd).as_deref(), Some("a\r\nb"));
    replace(hwnd, 2, 2, "x", "a\rx\nb");
    assert_eq!(fixture.native_text(0, 5), "a\r\nx\r\nb");
}

#[test]
fn native_bare_cr_keeps_source_offsets_when_window_text_expands_the_line_break() {
    let fixture = HiddenRichEdit::new("a\rb");
    let hwnd = fixture.edit;
    assert_eq!(length(hwnd), Some(3));
    assert_eq!(unsafe { GetWindowTextLengthW(hwnd) }, 4);
    select(hwnd, 2, 3);
    assert_eq!(selection(hwnd), (2, 3));
    assert_eq!(fixture.copied_model_text(), "b");
    assert_eq!(fixture.native_text(2, 3), "b");
    select(hwnd, 3, 3);
    assert_eq!(selection(hwnd), (3, 3));
    append(hwnd, "tail");
    assert_eq!(text(hwnd).as_deref(), Some("a\rbtail"));
    assert_eq!(length(hwnd), Some(7));
    assert_eq!(fixture.native_text(2, 7), "btail");
}

#[test]
fn native_batch_resume_follows_new_output_and_preserves_an_explicit_history_view() {
    use windows::Win32::UI::Controls::{EM_GETFIRSTVISIBLELINE, EM_LINESCROLL};
    use windows::Win32::UI::WindowsAndMessaging::WM_SETREDRAW;

    let source = (0..200)
        .map(|line| format!("line {line:03} Ж🙂e\u{301}\r\n"))
        .collect::<String>();
    let fixture = HiddenRichEdit::new(&source);
    let hwnd = fixture.edit;
    let first_line = || unsafe { SendMessageW(hwnd, EM_GETFIRSTVISIBLELINE, None, None).0 };
    select(hwnd, 0, 0);
    unsafe {
        SendMessageW(hwnd, EM_LINESCROLL, None, Some(LPARAM(-1_000)));
        SendMessageW(hwnd, WM_SETREDRAW, Some(WPARAM(0)), None);
    }
    assert_eq!(first_line(), 0);
    append(hwnd, "new result\r\n");
    let end = length(hwnd).unwrap();
    crate::finish_native_reading_update(hwnd, true, (end, end), end, true);
    assert!(
        first_line() > 150,
        "the new command result must be in view, first line={}",
        first_line()
    );
    assert_eq!(selection(hwnd), (end, end));
    let mut scroll = windows::Win32::UI::WindowsAndMessaging::SCROLLINFO {
        cbSize: std::mem::size_of::<windows::Win32::UI::WindowsAndMessaging::SCROLLINFO>() as u32,
        fMask: windows::Win32::UI::WindowsAndMessaging::SIF_RANGE
            | windows::Win32::UI::WindowsAndMessaging::SIF_PAGE
            | windows::Win32::UI::WindowsAndMessaging::SIF_POS,
        ..Default::default()
    };
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetScrollInfo(
            hwnd,
            windows::Win32::UI::WindowsAndMessaging::SB_VERT,
            &mut scroll,
        )
        .unwrap();
    }
    assert!(
        crate::reader_at_bottom(hwnd),
        "first={}, position={}, max={}, page={}",
        first_line(),
        scroll.nPos,
        scroll.nMax,
        scroll.nPage
    );

    let viewport = source[..source.find("line 020").unwrap()]
        .encode_utf16()
        .count() as u32;
    let selected = (viewport, viewport + 8);
    crate::restore_native_reading(hwnd, selected, viewport, false);
    let history_line = first_line();
    assert!((19..=21).contains(&history_line));
    unsafe {
        SendMessageW(hwnd, WM_SETREDRAW, Some(WPARAM(0)), None);
    }
    append(hwnd, "passive stream\r\n");
    crate::finish_native_reading_update(hwnd, true, selected, viewport, false);
    assert_eq!(first_line(), history_line);
    assert_eq!(selection(hwnd), selected);
    assert_eq!(fixture.copied_model_text(), "line 020");
}

#[test]
fn native_append_and_retention_rebase_styles_without_formatting_the_full_document() {
    let fixture = HiddenRichEdit::new("old\r\nЖ🙂");
    let hwnd = fixture.edit;
    let theme = Theme::default();
    let plain = TextStyle::default();
    let green = style((0x22, 0xCC, 0x44), (2, 3, 4), Flags::empty());
    assert_eq!(
        format(hwnd, &[span(0, 5, plain), span(5, 8, green)], &theme).unwrap(),
        2
    );
    let appended = "old\r\nЖ🙂\r\nnew";
    replace(hwnd, 8, 8, "\r\nnew", appended);
    select(hwnd, 5, 8);
    assert_eq!(
        format(
            hwnd,
            &[span(0, 5, plain), span(5, 8, green), span(8, 13, plain)],
            &theme
        )
        .unwrap(),
        1
    );
    assert_eq!(selection(hwnd), (5, 8));
    assert_eq!(fixture.native_text(0, 13), appended);
    replace(hwnd, 0, 5, "", "Ж🙂\r\nnew");
    select(hwnd, 0, 3);
    let retained = [span(0, 3, green), span(3, 8, plain)];
    assert_eq!(format(hwnd, &retained, &theme).unwrap(), 0);
    assert_eq!(selection(hwnd), (0, 3));
    assert_eq!(fixture.copied_model_text(), "Ж🙂");
    assert_eq!(fixture.native_text(0, 8), "Ж🙂\r\nnew");
    assert_eq!(
        unsafe { fixture.font(0, 3).GetForeColor().unwrap() },
        0x44_CC_22
    );
    set_text(hwnd, "reset");
    assert_eq!(format(hwnd, &[span(0, 5, plain)], &theme).unwrap(), 1);
    assert_eq!(fixture.native_text(0, 5), "reset");
}

#[derive(Debug)]
struct UiaSelection {
    text: String,
    foreground: i32,
    background: i32,
    line: String,
    next_line: String,
}

fn uia_selection_on_mta(hwnd: usize) -> Result<UiaSelection> {
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let _apartment = Apartment;
        let automation: IUIAutomation2 =
            CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)?;
        automation.SetConnectionTimeout(1_000)?;
        automation.SetTransactionTimeout(1_000)?;
        let element = automation.ElementFromHandle(HWND(hwnd as *mut std::ffi::c_void))?;
        let pattern: IUIAutomationTextPattern = element.GetCurrentPatternAs(UIA_TextPatternId)?;
        let ranges = pattern.GetSelection()?;
        let range = ranges.GetElement(0)?;
        let text = range.GetText(-1)?.to_string();
        let mut foreground = range.GetAttributeValue(UIA_ForegroundColorAttributeId)?;
        let mut background = range.GetAttributeValue(UIA_BackgroundColorAttributeId)?;
        let colors = (VariantToInt32(&foreground), VariantToInt32(&background));
        VariantClear(&mut foreground)?;
        VariantClear(&mut background)?;
        let line_range = range.Clone()?;
        line_range.ExpandToEnclosingUnit(windows::Win32::UI::Accessibility::TextUnit_Line)?;
        let line = line_range.GetText(-1)?.to_string();
        line_range.Move(windows::Win32::UI::Accessibility::TextUnit_Line, 1)?;
        let next_line = line_range.GetText(-1)?.to_string();
        Ok(UiaSelection {
            text,
            foreground: colors.0?,
            background: colors.1?,
            line,
            next_line,
        })
    }
}

fn read_uia_selection(hwnd: HWND) -> UiaSelection {
    let hwnd = hwnd.0 as usize;
    let (sender, receiver) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let result =
            uia_selection_on_mta(hwnd).map_err(|error| format!("UIA hidden RichEdit: {error}"));
        let _ = sender.send(result);
    });
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        match receiver.try_recv() {
            Ok(result) => {
                worker.join().expect("UIA worker panicked");
                return result.expect("native TextPattern and text attributes must be available");
            }
            Err(mpsc::TryRecvError::Disconnected) => panic!("UIA worker exited without a result"),
            Err(mpsc::TryRecvError::Empty) => {}
        }
        assert!(
            Instant::now() < deadline,
            "UIA hidden RichEdit exceeded the 8-second fixture deadline"
        );
        let mut message = MSG::default();
        unsafe {
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn native_model_menu_palette_refresh_keeps_text_caret_and_source_styles() {
    let mut terminal = bks_terminal::TerminalModel::new(110, 36);
    terminal.push_bytes(&include_bytes!("../tests/fixtures/tui-opencode-model-menu.vt")[..19_449]);
    let frame = terminal.live_snapshot();
    let original = frame.clone();
    let source = terminal.tui_reading_document();
    let visual = crate::tui_projection::visual_document(&frame, &source, &[]);
    let document = &visual.document;
    let source_styles = document.styles.clone();
    let selected = document
        .styles
        .iter()
        .find(|span| {
            matches!(
                span.style.background,
                Color::Spec(Rgb {
                    r: 250,
                    g: 178,
                    b: 131
                })
            ) && !document
                .text
                .encode_utf16()
                .skip(span.start as usize)
                .take((span.end - span.start) as usize)
                .all(|unit| unit == b' ' as u16)
        })
        .unwrap();
    let fixture = HiddenRichEdit::new(&document.text);
    let theme = Theme::default();
    let surface = TuiSurface::detect_grid(&frame).unwrap();
    select(fixture.edit, selected.start, selected.end);
    let copied = fixture.copied_model_text();
    assert!(
        format_tui(
            fixture.edit,
            &document.styles,
            &theme,
            visual.live_start(),
            Some(surface)
        )
        .unwrap()
            > 0
    );
    let expected = surface.resolve(selected.style, &theme);
    assert_eq!(
        unsafe {
            fixture
                .font(selected.start, selected.end)
                .GetBackColor()
                .unwrap()
        },
        expected.background.0 as i32
    );
    assert_eq!(
        unsafe {
            fixture
                .font(selected.start, selected.end)
                .GetForeColor()
                .unwrap()
        },
        expected.foreground.0 as i32
    );
    assert_eq!(selection(fixture.edit), (selected.start, selected.end));
    assert_eq!(fixture.copied_model_text(), copied);
    assert_eq!(
        format_tui(
            fixture.edit,
            &document.styles,
            &theme,
            visual.live_start(),
            Some(surface)
        )
        .unwrap(),
        0
    );
    // Identical source spans still need a palette refresh when the surrounding
    // geometry no longer proves a floating menu. No text replacement is needed.
    let mut ambiguous = frame;
    ambiguous.cells[0].bg = selected.style.background;
    let plain = TuiSurface::detect_grid(&ambiguous).unwrap();
    assert!(
        format_tui(
            fixture.edit,
            &document.styles,
            &theme,
            visual.live_start(),
            Some(plain)
        )
        .unwrap()
            > 0
    );
    assert_eq!(
        unsafe {
            fixture
                .font(selected.start, selected.end)
                .GetBackColor()
                .unwrap()
        },
        resolve(selected.style, &theme).background.0 as i32
    );
    assert_eq!(selection(fixture.edit), (selected.start, selected.end));
    assert_eq!(
        fixture.native_text(0, document.text.encode_utf16().count() as u32),
        document.text
    );
    assert_eq!(document.styles, source_styles);
    assert_eq!(terminal.live_snapshot(), original);
}

#[test]
fn native_reader_keeps_graphite_material_when_a_popup_splits_the_neutral_canvas() {
    let terminal = crate::text_style::tests::panelled_menu_model();
    let frame = terminal.live_snapshot();
    let source = terminal.tui_reading_document();
    let visual = crate::tui_projection::visual_document(&frame, &source, &[]);
    let fixture = HiddenRichEdit::new(&visual.document.text);
    let theme = Theme::default();
    let surface = TuiSurface::detect_grid(&frame).unwrap();
    select(fixture.edit, 1, 1);
    format_tui(
        fixture.edit,
        &visual.document.styles,
        &theme,
        visual.live_start(),
        Some(surface),
    )
    .unwrap();
    let mut canvas_spans = 0;
    for span in &visual.document.styles {
        if surface.is_canvas(span.style) && span.start < span.end {
            assert_eq!(
                unsafe { fixture.font(span.start, span.end).GetBackColor().unwrap() },
                theme.colors.terminal.0 as i32
            );
            canvas_spans += 1;
        }
    }
    assert!(canvas_spans > 5);
    assert_eq!(selection(fixture.edit), (1, 1));
    assert_eq!(
        fixture.native_text(0, visual.document.text.encode_utf16().count() as u32),
        visual.document.text
    );
}

#[test]
fn native_uia_reads_captured_tui_as_separate_lines_without_repeating_the_page() {
    let mut terminal = bks_terminal::TerminalModel::new(110, 36);
    terminal.push_bytes(
        &include_bytes!("../tests/fixtures/tui-turns-opencode-two-answers.vt")[..121_923],
    );
    let frame = terminal.live_snapshot();
    let raw = terminal.tui_reading_document();
    let projection = crate::tui_projection::project(&frame, &raw, None);
    let input = projection.input.unwrap();
    let source = projection.output;
    let visual = crate::tui_projection::visual_document(
        &frame,
        &source,
        &[(input.first_row, input.last_row)],
    );
    let fixture = HiddenRichEdit::new(&visual.document.text);
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::MoveWindow(fixture.edit, 0, 0, 2400, 900, false)
            .unwrap();
    }
    let theme = Theme::default();
    format_tui(
        fixture.edit,
        &visual.document.styles,
        &theme,
        visual.live_start(),
        None,
    )
    .unwrap();
    let units: Vec<_> = visual.document.text.encode_utf16().collect();
    let selected = visual.document.text.find("На модели DeepSeek").unwrap();
    let selected = visual.document.text[..selected].encode_utf16().count() as u32;
    let row = visual
        .document
        .lines
        .iter()
        .position(|line| line.start <= selected && selected < line.end)
        .unwrap();
    let text_for = |row: usize| {
        let line = visual.document.lines[row];
        String::from_utf16(&units[line.start as usize..line.end as usize]).unwrap()
    };
    let expected = text_for(row);
    let following = text_for(row + 1);
    select(fixture.edit, selected, selected);
    let before = selection(fixture.edit);
    let spoken = read_uia_selection(fixture.edit);
    assert_eq!(spoken.line.trim_end(), expected.trim_end());
    assert_eq!(spoken.next_line.trim_end(), following.trim_end());
    assert!(spoken.line.contains("На модели DeepSeek V4.1 Flash Free."));
    assert!(!spoken.line.contains("Build"));
    assert!(!spoken.line.contains("ctrl+p"));
    assert!(spoken.line.encode_utf16().count() <= 114);
    assert_eq!(spoken.background as u32, theme.colors.terminal.0);
    assert_eq!(
        selection(fixture.edit),
        before,
        "UIA reading does not move the native caret"
    );

    // Returning to the same view on a focus change needs neither replacement
    // nor reformatting, so line navigation keeps its existing native range.
    assert_eq!(
        format_tui(
            fixture.edit,
            &visual.document.styles,
            &theme,
            visual.live_start(),
            None,
        )
        .unwrap(),
        0
    );
    let again = read_uia_selection(fixture.edit);
    assert_eq!(again.line, spoken.line);
    assert_eq!(again.next_line, spoken.next_line);
    assert_eq!(
        text(fixture.edit).as_deref(),
        Some(visual.document.text.as_str())
    );
}

#[test]
fn native_uia_textpattern_exposes_the_actual_selected_foreground_and_background() {
    let fixture = HiddenRichEdit::new("color Ж🙂");
    let hwnd = fixture.edit;
    let expected = style((0x12, 0x34, 0x56), (1, 2, 3), Flags::BOLD);
    format(hwnd, &[span(0, 9, expected)], &Theme::default()).unwrap();
    select(hwnd, 6, 9);
    let result = read_uia_selection(hwnd);
    assert_eq!(result.text, "Ж🙂");
    assert_eq!(result.foreground, 0x56_34_12);
    assert_eq!(result.background, 0x03_02_01);
    assert_eq!(selection(hwnd), (6, 9));
    let changed = style((0xAA, 0xBB, 0xCC), (4, 5, 6), Flags::empty());
    assert_eq!(
        format(hwnd, &[span(0, 9, changed)], &Theme::default()).unwrap(),
        1
    );
    let result = read_uia_selection(hwnd);
    assert_eq!(result.text, "Ж🙂");
    assert_eq!(result.foreground, 0xCC_BB_AA);
    assert_eq!(result.background, 0x06_05_04);
    assert_eq!(selection(hwnd), (6, 9));
}

#[test]
fn streaming_format_does_not_reset_unchanged_native_background() {
    use std::cell::Cell;
    thread_local! {
        static BACKGROUND_UPDATES: Cell<usize> = const { Cell::new(0) };
    }
    unsafe extern "system" fn observe_background(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _: usize,
        _: usize,
    ) -> LRESULT {
        if message == EM_SETBKGNDCOLOR {
            BACKGROUND_UPDATES.set(BACKGROUND_UPDATES.get() + 1);
        }
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    }
    let fixture = HiddenRichEdit::new("");
    unsafe {
        assert!(
            SetWindowSubclass(fixture.edit, Some(observe_background), 0x424b_5342, 0).as_bool()
        );
    }
    let theme = Theme::default();
    BACKGROUND_UPDATES.set(0);
    for token in 0..32 {
        append(fixture.edit, &format!("token{token} "));
        let end = length(fixture.edit).unwrap();
        let spans = [span(0, end, TextStyle::default())];
        assert!(format(fixture.edit, &spans, &theme).unwrap() > 0);
    }
    assert_eq!(
        BACKGROUND_UPDATES.get(),
        1,
        "streaming changed glyphs, not the control background"
    );
    assert!(fixture
        .native_text(0, length(fixture.edit).unwrap())
        .ends_with("token31 "));
}
