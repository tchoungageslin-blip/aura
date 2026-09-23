/// Numeric identifier for a source file within a compilation session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(pub u32);

impl FileId {
    pub const SYNTHETIC: Self = Self(u32::MAX);
}

/// A half-open byte range `[start, end)` inside a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub file: FileId,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub const fn new(file: FileId, start: u32, end: u32) -> Self {
        Self { file, start, end }
    }

    /// Zero-width span at `pos` — used for "expected X here" diagnostics.
    pub const fn point(file: FileId, pos: u32) -> Self {
        Self {
            file,
            start: pos,
            end: pos,
        }
    }

    pub const fn len(self) -> u32 {
        self.end - self.start
    }

    pub const fn is_empty(self) -> bool {
        self.start >= self.end
    }

    /// Smallest span covering both `self` and `other`.
    ///
    /// # Panics
    /// Panics if `self` and `other` are in different files.
    #[must_use]
    pub fn merge(self, other: Span) -> Span {
        assert_eq!(self.file, other.file, "cannot merge spans across files");
        Span {
            file: self.file,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    pub const fn contains(self, offset: u32) -> bool {
        self.start <= offset && offset < self.end
    }

    /// Byte range for slicing source text.
    pub const fn range(self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }
}
