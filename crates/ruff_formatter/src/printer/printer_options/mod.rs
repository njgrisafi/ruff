use crate::{FormatOptions, IndentStyle, LineWidth, TabWidth};

/// Options that affect how the [`crate::Printer`] prints the format tokens
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct PrinterOptions {
    /// Width of a single tab character (does it equal 2, 4, ... spaces?)
    pub tab_width: TabWidth,

    /// What's the max width of a line. Defaults to 80
    pub print_width: PrintWidth,

    /// The type of line ending to apply to the printed input
    pub line_ending: LineEnding,

    /// Whether the printer should use tabs or spaces to indent code and if spaces, by how many.
    pub indent_style: IndentStyle,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct PrintWidth(u32);

impl PrintWidth {
    pub fn new(width: u32) -> Self {
        Self(width)
    }
}

impl Default for PrintWidth {
    fn default() -> Self {
        LineWidth::default().into()
    }
}

impl From<LineWidth> for PrintWidth {
    fn from(width: LineWidth) -> Self {
        Self(u32::from(u16::from(width)))
    }
}

impl From<PrintWidth> for usize {
    fn from(width: PrintWidth) -> Self {
        width.0 as usize
    }
}

impl From<PrintWidth> for u32 {
    fn from(width: PrintWidth) -> Self {
        width.0
    }
}

impl<'a, O> From<&'a O> for PrinterOptions
where
    O: FormatOptions,
{
    fn from(options: &'a O) -> Self {
        PrinterOptions::default()
            .with_indent(options.indent_style())
            .with_print_width(options.line_width().into())
    }
}

impl PrinterOptions {
    #[must_use]
    pub fn with_print_width(mut self, width: PrintWidth) -> Self {
        self.print_width = width;
        self
    }

    #[must_use]
    pub fn with_indent(mut self, style: IndentStyle) -> Self {
        self.indent_style = style;

        self
    }

    pub fn with_tab_width(mut self, tab_width: TabWidth) -> Self {
        self.tab_width = tab_width;

        self
    }

    pub(crate) fn indent_style(&self) -> IndentStyle {
        self.indent_style
    }

    /// Width of an indent in characters.
    pub(super) const fn indent_width(&self) -> u32 {
        match self.indent_style {
            IndentStyle::Tab => self.tab_width.value(),
            IndentStyle::Space(count) => count as u32,
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub enum LineEnding {
    ///  Line Feed only (\n), common on Linux and macOS as well as inside git repos
    #[default]
    LineFeed,

    /// Carriage Return + Line Feed characters (\r\n), common on Windows
    CarriageReturnLineFeed,

    /// Carriage Return character only (\r), used very rarely
    CarriageReturn,
}

impl LineEnding {
    #[inline]
    pub const fn as_str(&self) -> &'static str {
        match self {
            LineEnding::LineFeed => "\n",
            LineEnding::CarriageReturnLineFeed => "\r\n",
            LineEnding::CarriageReturn => "\r",
        }
    }
}
