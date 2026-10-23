//! Управление сеансами ConPTY для BKS Term.
//!
//! Реализация `PtySession` (CreatePseudoConsole, потоки ввода/вывода)
//! появится на Этапе 1 по плану разработки.

/// Размер псевдотерминала в символьных ячейках.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtySize {
    pub cols: u16,
    pub rows: u16,
}

impl PtySize {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self { cols, rows }
    }
}

impl Default for PtySize {
    /// Стандартный размер консоли Windows.
    fn default() -> Self {
        Self { cols: 80, rows: 24 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_size_is_standard_console() {
        assert_eq!(PtySize::default(), PtySize::new(80, 24));
    }
}
