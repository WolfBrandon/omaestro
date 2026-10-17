//! The queue's columns: which ones show (the list icon's dropdown), how wide the text columns
//! are (dragging the edges in the heading row), and where each one lands on screen.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Number,
    Source,
    Title,
    Artist,
    Album,
    /// Where the song comes from, in words: Local, Spotify or YouTube Music.
    SourceName,
    Rating,
    Format,
    Time,
}

impl Column {
    pub const ALL: [Column; 9] = [
        Column::Number,
        Column::Source,
        Column::Title,
        Column::Artist,
        Column::Album,
        Column::SourceName,
        Column::Rating,
        Column::Format,
        Column::Time,
    ];

    /// The name kept in the settings file.
    pub fn key(self) -> &'static str {
        match self {
            Column::Number => "number",
            Column::Source => "source",
            Column::Title => "title",
            Column::Artist => "artist",
            Column::Album => "album",
            Column::SourceName => "source-name",
            Column::Rating => "rating",
            Column::Format => "format",
            Column::Time => "time",
        }
    }

    pub fn heading(self) -> &'static str {
        match self {
            Column::Number => "#",
            Column::Source => "",
            Column::Title => "TITLE",
            Column::Artist => "ARTIST",
            Column::Album => "ALBUM",
            Column::SourceName => "SOURCE",
            Column::Rating => "RATING",
            Column::Format => "FORMAT",
            Column::Time => "TIME",
        }
    }

    /// What the show/hide menu calls it.
    pub fn label(self) -> &'static str {
        match self {
            Column::Number => "Number",
            Column::Source => "Source icon",
            Column::Title => "Title",
            Column::Artist => "Artist",
            Column::Album => "Album (year)",
            Column::SourceName => "Source",
            Column::Rating => "Rating (stars)",
            Column::Format => "Format",
            Column::Time => "Time",
        }
    }

    /// Title, artist and album share the space left over; their widths can be dragged.
    pub fn flexible(self) -> bool {
        matches!(self, Column::Title | Column::Artist | Column::Album)
    }

    /// Room taken by the other columns, including the gap after them.
    fn fixed(self) -> u16 {
        match self {
            Column::Number => 4,
            Column::Source => 3,
            Column::SourceName => 15,
            Column::Rating => 11,
            Column::Format => 9,
            Column::Time => 5,
            _ => 0,
        }
    }
}

/// Which columns show and how wide the text ones are. Kept in the settings file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct QueueColumns {
    /// Columns not shown, by name: "number", "source" (the icon), "artist", "album",
    /// "source-name", "rating", "format", "time".
    pub hidden: Vec<String>,
    /// Widths of the text columns, in characters. The last one shown takes whatever's left.
    pub title: u16,
    pub artist: u16,
    pub album: u16,
}

impl Default for QueueColumns {
    fn default() -> Self {
        QueueColumns {
            hidden: Vec::new(),
            title: 30,
            artist: 22,
            album: 28,
        }
    }
}

/// The narrowest a text column can be dragged.
pub const MIN_WIDTH: u16 = 6;

impl QueueColumns {
    /// The title always shows.
    pub fn shown(&self, column: Column) -> bool {
        column == Column::Title || !self.hidden.iter().any(|h| h == column.key())
    }

    pub fn toggle(&mut self, column: Column) {
        if column == Column::Title {
            return;
        }
        if self.shown(column) {
            self.hidden.push(column.key().to_string());
        } else {
            self.hidden.retain(|h| h != column.key());
        }
    }

    pub fn width(&self, column: Column) -> u16 {
        match column {
            Column::Title => self.title,
            Column::Artist => self.artist,
            Column::Album => self.album,
            _ => column.fixed(),
        }
        .max(MIN_WIDTH)
    }

    pub fn set_width(&mut self, column: Column, width: u16) {
        let width = width.max(MIN_WIDTH);
        match column {
            Column::Title => self.title = width,
            Column::Artist => self.artist = width,
            Column::Album => self.album = width,
            _ => {}
        }
    }
}

/// Where one column goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub column: Column,
    pub x: u16,
    pub width: u16,
}

/// The layout of a row from `left` to `right` (both inclusive): the ▶ marker's column and
/// each shown column. Text columns get exactly their set widths and the last one shown takes
/// whatever's left; only when the row is too narrow for that are they all scaled to fit.
pub fn layout(left: u16, right: u16, cols: &QueueColumns) -> (u16, Vec<Placed>) {
    let total = (right + 1).saturating_sub(left);
    let shown: Vec<Column> = Column::ALL.into_iter().filter(|c| cols.shown(*c)).collect();
    let flexible: Vec<Column> = shown.iter().copied().filter(|c| c.flexible()).collect();
    let fixed: u16 = shown.iter().map(|c| c.fixed()).sum::<u16>() + 2; // + the ▶ marker
    let room = total.saturating_sub(fixed + 2 * flexible.len() as u16);
    let set: u32 = flexible.iter().map(|c| cols.width(*c) as u32).sum();
    let all_but_last = set - flexible.last().map_or(0, |c| cols.width(*c) as u32);
    let fits = all_but_last + MIN_WIDTH as u32 <= room as u32;
    let mut widths: Vec<u16> = flexible
        .iter()
        .map(|c| {
            let w = cols.width(*c) as u32;
            if fits {
                w as u16
            } else {
                (w * room as u32 / set.max(1)) as u16
            }
        })
        .collect();
    if let Some(n) = widths.len().checked_sub(1) {
        let others: u16 = widths[..n].iter().sum();
        widths[n] = room.saturating_sub(others);
    }

    let mut x = left;
    let mut marker = None;
    let mut placed = Vec::new();
    let mut flex = 0;
    for column in shown {
        if column == Column::Number {
            placed.push(Placed {
                column,
                x,
                width: 3,
            });
            x += column.fixed();
            continue;
        }
        // The ▶ goes just before the first column after the number.
        if marker.is_none() {
            marker = Some(x);
            x += 2;
        }
        let (width, gap) = match column {
            Column::Source => (1, 2),
            Column::SourceName => (13, 2),
            Column::Rating => (9, 2),
            Column::Format => (7, 2),
            Column::Time => (5, 0),
            _ => {
                flex += 1;
                (widths[flex - 1], 2)
            }
        };
        placed.push(Placed { column, x, width });
        x += width + gap;
    }
    (marker.unwrap_or(x), placed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(placed: &[Placed], c: Column) -> Option<Placed> {
        placed.iter().copied().find(|p| p.column == c)
    }

    #[test]
    fn columns_fill_the_row_and_the_last_text_column_takes_the_rest() {
        let cols = QueueColumns::default();
        let (marker, placed) = layout(4, 130, &cols);
        assert_eq!(find(&placed, Column::Number).unwrap().x, 4);
        assert_eq!(marker, 8);
        let title = find(&placed, Column::Title).unwrap();
        assert_eq!(title.width, 30);
        let time = find(&placed, Column::Time).unwrap();
        assert_eq!(time.x + time.width - 1, 130, "time ends at the right edge");
        let album = find(&placed, Column::Album).unwrap();
        let source = find(&placed, Column::SourceName).unwrap();
        assert_eq!(
            album.x + album.width + 2,
            source.x,
            "album grew to meet the source"
        );
        assert_eq!(source.width, "YouTube Music".len() as u16);
        let rating = find(&placed, Column::Rating).unwrap();
        assert_eq!(source.x + 15, rating.x);
        let format = find(&placed, Column::Format).unwrap();
        assert_eq!(rating.x + 11, format.x);
    }

    #[test]
    fn hiding_columns_and_narrow_rows() {
        let mut cols = QueueColumns::default();
        cols.toggle(Column::Album);
        cols.toggle(Column::Number);
        cols.toggle(Column::Title); // can't hide the title
        assert!(!cols.shown(Column::Album) && cols.shown(Column::Title));
        let (marker, placed) = layout(0, 79, &cols);
        assert_eq!(marker, 0);
        assert!(find(&placed, Column::Album).is_none());
        assert_eq!(
            find(&placed, Column::Time).map(|t| t.x + t.width - 1),
            Some(79)
        );

        // Too narrow for the widths asked: text columns shrink to fit.
        let cols = QueueColumns::default();
        let (_, placed) = layout(0, 59, &cols);
        let used: u16 = placed.iter().map(|p| p.width).sum();
        assert!(used <= 60, "{placed:?}");
        let mut c = QueueColumns::default();
        c.set_width(Column::Artist, 2);
        assert_eq!(c.width(Column::Artist), MIN_WIDTH);
    }

    #[test]
    fn widening_a_column_moves_its_edge_exactly_and_the_last_one_gives_way() {
        let mut cols = QueueColumns::default();
        let (_, before) = layout(4, 130, &cols);
        cols.set_width(Column::Title, 40);
        let (_, after) = layout(4, 130, &cols);
        assert_eq!(find(&after, Column::Title).unwrap().width, 40);
        let artist = |p: &[Placed]| find(p, Column::Artist).unwrap();
        assert_eq!(artist(&after).x, artist(&before).x + 10);
        assert_eq!(artist(&after).width, 22);
        let album = |p: &[Placed]| find(p, Column::Album).unwrap().width;
        assert_eq!(album(&after), album(&before) - 10);
    }
}
