//! Dropdown rows as plain Win32 menu text. A menu item draws what follows a
//! tab right-aligned in its own column, and the menu font (Segoe UI) has
//! tabular digits with a figure space as wide as one, so padding each change
//! to the widest with figure spaces lines the prices up digit by digit, as
//! the macOS dropdown's tab stops do.

use crate::bar::Row;

const FIGURE_SPACE: char = '\u{2007}';
const EM_SPACE: char = '\u{2003}';

/// `&` marks a mnemonic in menu text; a literal one is doubled.
pub fn escape(text: &str) -> String {
    text.replace('&', "&&")
}

/// `BTC/USDT⇥84,002.01 −0.24%`, `AAPL 苹果  盘后⇥182.33 +0.12%`: the name,
/// its detail and session left, value and change right. Rows laid out
/// together share one change column, so their values line up.
pub fn rows(rows: &[Row]) -> Vec<String> {
    let width = |row: &Row| row.change.as_ref().map_or(0, |(change, _)| change.chars().count());
    let change_width = rows.iter().map(width).max().unwrap_or(0);
    rows.iter()
        .map(|row| {
            let mut text = escape(&format!("{}{}", row.name, row.detail));
            if let Some(session) = row.session {
                text.push_str("  ");
                text.push_str(session);
            }
            text.push('\t');
            text.push_str(&row.value.0);
            if change_width > 0 {
                text.push(EM_SPACE);
                text.extend(std::iter::repeat_n(FIGURE_SPACE, change_width - width(row)));
                if let Some((change, _)) = &row.change {
                    text.push_str(change);
                }
            }
            text
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::Direction;

    fn row(name: &str, price: &str, change: Option<&str>) -> Row {
        Row {
            name: name.to_owned(),
            detail: String::new(),
            value: (price.to_owned(), Direction::Flat),
            session: None,
            change: change.map(|c| (c.to_owned(), Direction::Up)),
        }
    }

    // Win32 menus render the text after a tab right-aligned, so every row's
    // right part must be equally wide for the prices to share a right edge.
    #[test]
    fn changes_are_padded_to_one_width() {
        let texts = rows(&[
            row("BTC/USDT", "84,002.01", Some("+1.23%")),
            row("PEPE/USDT", "0.00001234", Some("−12.40%")),
            row("NEW/USDT", "—", None),
        ]);
        assert_eq!(texts[0], "BTC/USDT\t84,002.01\u{2003}\u{2007}+1.23%");
        assert_eq!(texts[1], "PEPE/USDT\t0.00001234\u{2003}−12.40%");
        assert_eq!(texts[2], format!("NEW/USDT\t—\u{2003}{}", "\u{2007}".repeat(7)));
        let change_column =
            |text: &String| text.split_once(EM_SPACE).map(|(_, tail)| tail.chars().count());
        assert!(texts.iter().all(|text| change_column(text) == Some(7)));
    }

    // Rows without any change (PnL rows) keep just the value column.
    #[test]
    fn values_alone_take_no_change_column() {
        let texts = rows(&[row("24h 盈亏", "+152.30", None)]);
        assert_eq!(texts[0], "24h 盈亏\t+152.30");
    }

    // AppendMenu treats `&` as a mnemonic prefix (MF_STRING docs); names such
    // as AT&T must show their ampersand.
    #[test]
    fn ampersands_are_literal() {
        assert_eq!(escape("AT&T"), "AT&&T");
        let texts = rows(&[Row { detail: " S&P".to_owned(), ..row("SPY", "500.00", None) }]);
        assert_eq!(texts[0], "SPY S&&P\t500.00");
    }
}
