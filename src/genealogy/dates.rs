//! Explicit date extraction for catalog facts (TRAN-002H).
//!
//! The parser records a birth or death date only when the source states one
//! explicitly. This module refuses to guess:
//!
//! - no locale guessing: `03/04/1842` is ambiguous, while `15/03/1842` and
//!   `1842-03-15` are unambiguous because only one reading is possible;
//! - no calendar guessing: an impossible day (`31/02/1842`) is rejected;
//! - no component guessing: a lone unlabelled year is not a birth or a death
//!   date, and is reported as unresolved by the caller;
//! - no plausibility guessing: a year outside [`MIN_YEAR`]..=[`MAX_YEAR`] is
//!   rejected rather than reinterpreted.
//!
//! Dates are stored as partial ISO-8601 text (`YYYY`, `YYYY-MM`, or
//! `YYYY-MM-DD`), which sorts chronologically as text and cannot silently
//! become a made-up day or month. The original wording is preserved alongside
//! the parsed components, so precision and uncertainty survive into the API.

/// The smallest year accepted as a catalog date.
pub const MIN_YEAR: u32 = 1000;
/// The largest year accepted as a catalog date.
pub const MAX_YEAR: u32 = 2100;

/// The components a stored date actually pins down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatePrecision {
    /// The source gives a full calendar day.
    Day,
    /// The source gives a month and a year.
    Month,
    /// The source gives a year only.
    Year,
    /// The source gives a range (`start` and `end`).
    Range,
}

impl DatePrecision {
    /// The precision a stored partial ISO value represents.
    pub fn of_iso(value: &str) -> DatePrecision {
        match value.len() {
            10 => DatePrecision::Day,
            7 => DatePrecision::Month,
            _ => DatePrecision::Year,
        }
    }

    /// The database text value.
    pub fn as_str(self) -> &'static str {
        match self {
            DatePrecision::Day => "day",
            DatePrecision::Month => "month",
            DatePrecision::Year => "year",
            DatePrecision::Range => "range",
        }
    }

    /// Parses a database text value.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "day" => DatePrecision::Day,
            "month" => DatePrecision::Month,
            "year" => DatePrecision::Year,
            "range" => DatePrecision::Range,
            _ => return None,
        })
    }
}

/// How certain the source is about the stated date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateQualifier {
    /// Stated without hedging.
    Exact,
    /// Hedged as approximate (`c.`, `khoảng`, `~`, `?`).
    Circa,
    /// An upper bound ("before 1842").
    Before,
    /// A lower bound ("after 1842").
    After,
}

impl DateQualifier {
    /// The database text value.
    pub fn as_str(self) -> &'static str {
        match self {
            DateQualifier::Exact => "exact",
            DateQualifier::Circa => "circa",
            DateQualifier::Before => "before",
            DateQualifier::After => "after",
        }
    }

    /// Parses a database text value.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "exact" => DateQualifier::Exact,
            "circa" => DateQualifier::Circa,
            "before" => DateQualifier::Before,
            "after" => DateQualifier::After,
            _ => return None,
        })
    }
}

/// One explicitly stated date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateValue {
    /// The ISO start value: `YYYY`, `YYYY-MM`, or `YYYY-MM-DD`.
    pub start: String,
    /// The ISO end value, present exactly when the precision is a range.
    pub end: Option<String>,
    /// The precision the source supports.
    pub precision: DatePrecision,
    /// The source's own certainty.
    pub qualifier: DateQualifier,
    /// The original wording, so the claim keeps its provenance.
    pub raw: String,
}

impl DateValue {
    /// Whether the components and the precision agree.
    pub fn is_consistent(&self) -> bool {
        let range = self.precision == DatePrecision::Range;
        range == self.end.is_some()
            && is_iso_partial(&self.start)
            && self.end.as_deref().is_none_or(is_iso_partial)
    }
}

/// Why a date-like expression was not turned into a [`DateValue`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmbiguousReason {
    /// A numeric component could be a day or a month.
    AmbiguousComponent,
    /// A range whose end does not follow its start.
    InvertedRange,
    /// A date with no birth/death label, so its meaning is unknown.
    Unlabeled,
    /// A year outside the accepted catalog range.
    OutOfRange,
    /// An impossible calendar date.
    InvalidDate,
    /// Text that looks like a date but cannot be read.
    Unparseable,
}

impl AmbiguousReason {
    /// The stable text value used in reports.
    pub fn as_str(self) -> &'static str {
        match self {
            AmbiguousReason::AmbiguousComponent => "ambiguous_component",
            AmbiguousReason::InvertedRange => "inverted_range",
            AmbiguousReason::Unlabeled => "unlabeled",
            AmbiguousReason::OutOfRange => "out_of_range",
            AmbiguousReason::InvalidDate => "invalid_date",
            AmbiguousReason::Unparseable => "unparseable",
        }
    }
}

/// The result of reading a date expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DateParse {
    /// An explicitly stated date.
    Date(DateValue),
    /// A date-like expression that needs a reviewer, never a guess.
    Ambiguous {
        /// The text that was not resolved.
        raw: String,
        /// Why it was not resolved.
        reason: AmbiguousReason,
    },
    /// No date expression at the start of the text.
    None,
}

/// A single parsed date component set, before range handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SingleDate {
    year: u32,
    month: Option<u32>,
    day: Option<u32>,
    /// Bytes consumed from the input.
    len: usize,
}

impl SingleDate {
    fn iso(&self) -> String {
        match (self.month, self.day) {
            (Some(month), Some(day)) => format!("{:04}-{month:02}-{day:02}", self.year),
            (Some(month), None) => format!("{:04}-{month:02}", self.year),
            _ => format!("{:04}", self.year),
        }
    }

    fn precision(&self) -> DatePrecision {
        match (self.month, self.day) {
            (Some(_), Some(_)) => DatePrecision::Day,
            (Some(_), None) => DatePrecision::Month,
            _ => DatePrecision::Year,
        }
    }
}

/// Parses a whole date expression, for example `1877–1922` or `c. 15/03/1842`.
///
/// Trailing prose is ignored once a complete date has been read, so a field
/// such as `born 1877 in Hà Nội` still yields the year.
pub fn parse_date_expression(input: &str) -> DateParse {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return DateParse::None;
    }

    let (qualifier, body) = split_qualifier(trimmed);
    let body = body.trim();
    if body.is_empty() {
        return DateParse::None;
    }

    // A trailing question mark also means "approximate", never "exact".
    let (body, trailing_uncertain) = match body.strip_suffix('?') {
        Some(rest) => (rest.trim_end(), true),
        None => (body, false),
    };
    let qualifier = if trailing_uncertain && qualifier == DateQualifier::Exact {
        DateQualifier::Circa
    } else {
        qualifier
    };

    let first = match parse_single_prefix(body) {
        None if starts_with_digit(body) => {
            return DateParse::Ambiguous {
                raw: trimmed.to_string(),
                reason: AmbiguousReason::Unparseable,
            }
        }
        None => return DateParse::None,
        Some(Err(reason)) => {
            return DateParse::Ambiguous {
                raw: trimmed.to_string(),
                reason,
            }
        }
        Some(Ok(value)) => value,
    };
    let first = match validate_single(first) {
        Ok(value) => value,
        Err(reason) => {
            return DateParse::Ambiguous {
                raw: trimmed.to_string(),
                reason,
            }
        }
    };

    let rest = body[first.len..].trim_start_matches([' ', '\t']);
    if rest.is_empty() {
        return DateParse::Date(DateValue {
            start: first.iso(),
            end: None,
            precision: first.precision(),
            qualifier,
            raw: trimmed.to_string(),
        });
    }

    let Some(after_sep) = take_range_separator(rest) else {
        // A hyphen plus digits that is not a valid range is a malformed date,
        // not a bare year with trailing punctuation.
        if rest.starts_with('-') && starts_with_digit(&rest[1..]) {
            return DateParse::Ambiguous {
                raw: trimmed.to_string(),
                reason: AmbiguousReason::Unparseable,
            };
        }
        // Trailing prose after a complete date is not part of the date.
        return DateParse::Date(DateValue {
            start: first.iso(),
            end: None,
            precision: first.precision(),
            qualifier,
            raw: trimmed.to_string(),
        });
    };

    let second_source = after_sep.trim_start_matches([' ', '\t']);
    let second = match parse_single_prefix(second_source) {
        None | Some(Err(_)) => {
            return DateParse::Ambiguous {
                raw: trimmed.to_string(),
                reason: AmbiguousReason::Unparseable,
            }
        }
        Some(Ok(value)) => value,
    };
    let second = match validate_single(second) {
        Ok(value) => value,
        Err(reason) => {
            return DateParse::Ambiguous {
                raw: trimmed.to_string(),
                reason,
            }
        }
    };

    let start = first.iso();
    let end = second.iso();
    if end <= start {
        return DateParse::Ambiguous {
            raw: trimmed.to_string(),
            reason: AmbiguousReason::InvertedRange,
        };
    }

    DateParse::Date(DateValue {
        start,
        end: Some(end),
        precision: DatePrecision::Range,
        qualifier,
        raw: trimmed.to_string(),
    })
}

/// Reads a leading date expression from a labelled field.
///
/// Used for fields such as `born 1877 in Hà Nội`: the date is taken from the
/// start, and the remaining text is not reinterpreted. A range is *not*
/// accepted here, because `born 1877–1880` and similar fields are read by the
/// caller that knows which side is the birth and which is the death.
pub fn take_leading_date(input: &str) -> DateParse {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return DateParse::None;
    }
    let (qualifier, body) = split_qualifier(trimmed);
    let body = body.trim_start();
    if body.is_empty() {
        return DateParse::None;
    }
    match parse_single_prefix(body) {
        None if starts_with_digit(body) => DateParse::Ambiguous {
            raw: trimmed.to_string(),
            reason: AmbiguousReason::Unparseable,
        },
        None => DateParse::None,
        Some(Err(reason)) => DateParse::Ambiguous {
            raw: trimmed.to_string(),
            reason,
        },
        Some(Ok(value)) => match validate_single(value) {
            Ok(value) => DateParse::Date(DateValue {
                start: value.iso(),
                end: None,
                precision: value.precision(),
                qualifier,
                raw: trimmed.to_string(),
            }),
            Err(reason) => DateParse::Ambiguous {
                raw: trimmed.to_string(),
                reason,
            },
        },
    }
}

/// Strips a leading certainty marker, returning it with the remaining text.
///
/// Public so the catalog parser can recognise a bare date clause such as
/// `c. 1914` without re-implementing the marker list.
pub fn split_qualifier(text: &str) -> (DateQualifier, &str) {
    // Longest markers first, so `circa` is not read as `c.`.
    const CIRCA: [&str; 9] = [
        "approximately",
        "approx.",
        "approx",
        "circa",
        "about",
        "khoảng",
        "khoang",
        "ca.",
        "c.",
    ];
    for marker in CIRCA {
        if let Some(rest) = strip_prefix_ci(text, marker) {
            let rest = rest.trim_start_matches([' ', '\t']);
            if starts_with_date_like(rest) {
                return (DateQualifier::Circa, rest);
            }
        }
    }
    if let Some(rest) = text.strip_prefix('~') {
        let rest = rest.trim_start_matches([' ', '\t']);
        if starts_with_date_like(rest) {
            return (DateQualifier::Circa, rest);
        }
    }
    for marker in ["before", "trước", "truoc"] {
        if let Some(rest) = strip_prefix_ci(text, marker) {
            return (DateQualifier::Before, rest.trim_start_matches([' ', '\t']));
        }
    }
    for marker in ["after", "sau"] {
        if let Some(rest) = strip_prefix_ci(text, marker) {
            return (DateQualifier::After, rest.trim_start_matches([' ', '\t']));
        }
    }
    (DateQualifier::Exact, text)
}

/// Case-insensitive prefix removal that is safe for non-ASCII markers.
///
/// The prefix is taken by *character count* from `text`, so byte-length changes
/// during lowercasing (`İ`, for example) cannot produce an invalid boundary.
fn strip_prefix_ci<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    let wanted = marker.chars().count();
    let end = text
        .char_indices()
        .nth(wanted)
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    let prefix = &text[..end];
    if prefix.to_lowercase() == marker.to_lowercase() {
        Some(&text[end..])
    } else {
        None
    }
}

fn starts_with_digit(text: &str) -> bool {
    text.chars().next().is_some_and(|ch| ch.is_ascii_digit())
}

fn starts_with_date_like(text: &str) -> bool {
    text.chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_digit() || ch == '~')
}

/// Consumes a range separator, returning the text after it.
fn take_range_separator(rest: &str) -> Option<&str> {
    let rest = rest.trim_start_matches([' ', '\t']);
    if let Some(tail) = rest.strip_prefix('–').or_else(|| rest.strip_prefix('—')) {
        return Some(tail);
    }
    if let Some(tail) = rest.strip_prefix("..") {
        return Some(tail);
    }
    for marker in ["to ", "until ", "through ", "đến ", "den ", "/ "] {
        if let Some(tail) = strip_prefix_ci(rest, marker) {
            return Some(tail);
        }
    }
    // A plain hyphen is only a range between two four-digit years, so an ISO
    // date or a day/month/year date is never split in the wrong place.
    if let Some(tail) = rest.strip_prefix('-') {
        let digits: String = tail.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if digits.len() == 4 {
            return Some(tail);
        }
    }
    None
}

/// Reads a date from the start of `text`.
///
/// `None` means "not a date expression"; `Some(Err(..))` means "date-like but
/// not resolvable", which the caller must keep for review.
fn parse_single_prefix(text: &str) -> Option<Result<SingleDate, AmbiguousReason>> {
    let (first, first_len) = leading_digits(text, 4);
    if first_len == 0 {
        return None;
    }
    let after_first = &text[first_len..];

    if first_len == 4 {
        // A fifth digit means this is not a year token at all.
        if starts_with_digit(after_first) {
            return None;
        }
        let mut len = first_len;
        let mut month = None;
        let mut day = None;
        if let Some(tail) = after_first.strip_prefix('-') {
            let (candidate_month, month_len) = leading_digits(tail, 2);
            if month_len == 2 && month_valid(candidate_month) {
                let after_month = &tail[month_len..];
                if starts_with_digit(after_month) {
                    return None;
                }
                month = Some(candidate_month);
                len += 1 + month_len;
                if let Some(tail) = after_month.strip_prefix('-') {
                    let (candidate_day, day_len) = leading_digits(tail, 2);
                    if day_len != 2 {
                        return Some(Err(AmbiguousReason::Unparseable));
                    }
                    day = Some(candidate_day);
                    len += 1 + day_len;
                }
            }
            // An invalid month leaves the hyphen to the range parser, so
            // `1877-1922` stays a range while `1842-13-01` is refused.
        }
        if text[len..]
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_digit())
        {
            return None;
        }
        return Some(Ok(SingleDate {
            year: first,
            month,
            day,
            len,
        }));
    }

    // A one- or two-digit value opens a day/month/year form.
    let separator = after_first
        .chars()
        .next()
        .filter(|ch| matches!(ch, '/' | '.' | '-'))?;
    let tail = &after_first[separator.len_utf8()..];
    let (second, second_len) = leading_digits(tail, 4);
    if second_len == 0 {
        return None;
    }
    if second_len == 4 {
        // `5/1842`: a month and a year, with no day. The four-digit value is the
        // year, so the leading component is the month.
        if first > 12 {
            return Some(Err(AmbiguousReason::AmbiguousComponent));
        }
        let len = first_len + separator.len_utf8() + second_len;
        if text[len..]
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_digit())
        {
            return None;
        }
        return Some(Ok(SingleDate {
            year: second,
            month: Some(first),
            day: None,
            len,
        }));
    }

    if second_len > 2 {
        return None;
    }
    let after_second = &tail[second_len..];
    let second_separator = after_second
        .chars()
        .next()
        .filter(|ch| matches!(ch, '/' | '.' | '-'))?;
    let tail = &after_second[second_separator.len_utf8()..];
    let (year, year_len) = leading_digits(tail, 4);
    if year_len != 4 {
        return None;
    }
    let len =
        first_len + separator.len_utf8() + second_len + second_separator.len_utf8() + year_len;
    if text[len..]
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_digit())
    {
        return None;
    }

    // Only a forced reading is accepted. When both leading components could be
    // a day or a month, the order is a locale question this parser refuses to
    // answer, so the claim stays unresolved.
    let (day, month) = if first > 12 && second <= 12 {
        (Some(first), Some(second))
    } else if second > 12 && first <= 12 {
        (Some(second), Some(first))
    } else if first > 12 && second > 12 {
        return Some(Err(AmbiguousReason::InvalidDate));
    } else {
        return Some(Err(AmbiguousReason::AmbiguousComponent));
    };

    Some(Ok(SingleDate {
        year,
        month,
        day,
        len,
    }))
}

/// Rejects impossible components without inventing a replacement.
fn validate_single(value: SingleDate) -> Result<SingleDate, AmbiguousReason> {
    if !(MIN_YEAR..=MAX_YEAR).contains(&value.year) {
        return Err(AmbiguousReason::OutOfRange);
    }
    if let Some(month) = value.month {
        if !month_valid(month) {
            return Err(AmbiguousReason::InvalidDate);
        }
        if let Some(day) = value.day {
            if day == 0 || day > days_in_month(value.year, month) {
                return Err(AmbiguousReason::InvalidDate);
            }
        }
    }
    Ok(value)
}

fn month_valid(month: u32) -> bool {
    (1..=12).contains(&month)
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

/// Reads at most `max` leading ASCII digits.
fn leading_digits(text: &str, max: usize) -> (u32, usize) {
    let mut value = 0u32;
    let mut len = 0usize;
    for ch in text.chars() {
        if len == max || !ch.is_ascii_digit() {
            break;
        }
        value = value
            .saturating_mul(10)
            .saturating_add(ch as u32 - '0' as u32);
        len += ch.len_utf8();
    }
    (value, len)
}

/// The inclusive day range a partial ISO value covers, as `YYYYMMDD` numbers.
///
/// A year covers the whole year and a month covers the whole month, so two
/// claims about the same person can be compared without inventing a day.
pub fn iso_bounds(value: &str) -> Option<(i32, i32)> {
    if !is_iso_partial(value) {
        return None;
    }
    let year: i32 = value[..4].parse().ok()?;
    let start = match value.len() {
        4 => year * 10_000 + 101,
        7 => {
            let month: i32 = value[5..].parse().ok()?;
            year * 10_000 + month * 100 + 1
        }
        _ => {
            let month: i32 = value[5..7].parse().ok()?;
            let day: i32 = value[8..].parse().ok()?;
            year * 10_000 + month * 100 + day
        }
    };
    let end = match value.len() {
        4 => year * 10_000 + 1231,
        7 => {
            let month: i32 = value[5..].parse().ok()?;
            year * 10_000 + month * 100 + days_in_month(year as u32, month as u32) as i32
        }
        _ => start,
    };
    Some((start, end))
}

/// The inclusive day range of one stored claim (`start` plus an optional range
/// end). Returns `None` when either value is not a partial ISO date.
pub fn claim_bounds(start: &str, end: Option<&str>) -> Option<(i32, i32)> {
    let (lower, _) = iso_bounds(start)?;
    let upper = match end {
        Some(end) => iso_bounds(end)?.1,
        None => iso_bounds(start)?.1,
    };
    Some((lower, upper))
}

/// How much a claim pins down. Used only to prefer the more precise of two
/// claims that already overlap; it never breaks a real disagreement.
pub fn precision_rank(precision: DatePrecision) -> u8 {
    match precision {
        DatePrecision::Day => 3,
        DatePrecision::Month => 2,
        DatePrecision::Year => 1,
        DatePrecision::Range => 0,
    }
}

/// Chooses which of several verified claims to present.
///
/// Returns `None` when any two claims disagree, so a caller never invents a
/// winner between contradictory dates. Otherwise returns the index of the most
/// precise claim, keeping the first one on a tie.
pub fn representative_claim(claims: &[(String, Option<String>, DatePrecision)]) -> Option<usize> {
    if claims.len() <= 1 {
        return (!claims.is_empty()).then_some(0);
    }
    let mut best: Option<usize> = None;
    for (index, claim) in claims.iter().enumerate() {
        let bounds = claim_bounds(&claim.0, claim.1.as_deref())?;
        for other in claims {
            let other_bounds = claim_bounds(&other.0, other.1.as_deref())?;
            if !intervals_overlap(bounds, other_bounds) {
                return None;
            }
        }
        let rank = precision_rank(claim.2);
        best = match best {
            Some(current) if precision_rank(claims[current].2) >= rank => Some(current),
            _ => Some(index),
        };
    }
    best
}

/// Whether two date claims can both be true.
///
/// Overlapping claims are refinements of one another (`1877` and `1877-03`);
/// disjoint claims are a real disagreement and must go back to review.
pub fn intervals_overlap(left: (i32, i32), right: (i32, i32)) -> bool {
    left.0 <= right.1 && right.0 <= left.1
}

/// Whether a value is a partial ISO-8601 date (`YYYY`, `YYYY-MM`, `YYYY-MM-DD`).
pub fn is_iso_partial(value: &str) -> bool {
    let bytes = value.as_bytes();
    if !bytes
        .iter()
        .enumerate()
        .all(|(index, byte)| byte.is_ascii_digit() || matches!(index, 4 | 7) && *byte == b'-')
    {
        return false;
    }
    let Ok(_year) = value[..4.min(value.len())].parse::<u32>() else {
        return false;
    };
    match bytes.len() {
        4 => true,
        7 => {
            if bytes[4] != b'-' {
                return false;
            }
            let Ok(month) = value[5..].parse::<u32>() else {
                return false;
            };
            month_valid(month)
        }
        10 => {
            if bytes[4] != b'-' || bytes[7] != b'-' {
                return false;
            }
            let year = value[..4].parse::<u32>().unwrap_or(0);
            let (Ok(month), Ok(day)) = (value[5..7].parse::<u32>(), value[8..].parse::<u32>())
            else {
                return false;
            };
            month_valid(month) && day >= 1 && day <= days_in_month(year, month)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(parse: DateParse) -> DateValue {
        match parse {
            DateParse::Date(value) => value,
            other => panic!("expected a date, got {other:?}"),
        }
    }

    fn reason(parse: DateParse) -> AmbiguousReason {
        match parse {
            DateParse::Ambiguous { reason, .. } => reason,
            other => panic!("expected an ambiguity, got {other:?}"),
        }
    }

    #[test]
    fn full_iso_date_keeps_day_precision() {
        let value = date(parse_date_expression("1842-03-15"));
        assert_eq!(value.start, "1842-03-15");
        assert_eq!(value.precision, DatePrecision::Day);
        assert_eq!(value.qualifier, DateQualifier::Exact);
        assert!(value.is_consistent());
    }

    #[test]
    fn forced_day_first_date_is_accepted() {
        let value = date(parse_date_expression("15/03/1842"));
        assert_eq!(value.start, "1842-03-15");
        assert_eq!(value.precision, DatePrecision::Day);
    }

    #[test]
    fn month_first_is_accepted_only_when_forced() {
        let value = date(parse_date_expression("03/15/1842"));
        assert_eq!(value.start, "1842-03-15");
    }

    #[test]
    fn ambiguous_numeric_order_is_refused() {
        assert_eq!(
            reason(parse_date_expression("03/04/1842")),
            AmbiguousReason::AmbiguousComponent
        );
    }

    #[test]
    fn year_and_month_precisions_are_kept() {
        assert_eq!(
            date(parse_date_expression("1842")).precision,
            DatePrecision::Year
        );
        assert_eq!(
            date(parse_date_expression("1842-03")).precision,
            DatePrecision::Month
        );
    }

    #[test]
    fn approximate_markers_are_recorded_not_dropped() {
        let value = date(parse_date_expression("c. 1842"));
        assert_eq!(value.start, "1842");
        assert_eq!(value.qualifier, DateQualifier::Circa);
        assert_eq!(
            date(parse_date_expression("khoảng 1842")).qualifier,
            DateQualifier::Circa
        );
        assert_eq!(
            date(parse_date_expression("1842?")).qualifier,
            DateQualifier::Circa
        );
        assert_eq!(
            date(parse_date_expression("~1842")).qualifier,
            DateQualifier::Circa
        );
    }

    #[test]
    fn bounds_are_recorded() {
        assert_eq!(
            date(parse_date_expression("before 1842")).qualifier,
            DateQualifier::Before
        );
        assert_eq!(
            date(parse_date_expression("after 1842")).qualifier,
            DateQualifier::After
        );
        assert_eq!(
            date(parse_date_expression("1842")).qualifier,
            DateQualifier::Exact
        );
    }

    #[test]
    fn year_ranges_are_ranges() {
        let value = date(parse_date_expression("1877–1922"));
        assert_eq!(value.start, "1877");
        assert_eq!(value.end.as_deref(), Some("1922"));
        assert_eq!(value.precision, DatePrecision::Range);
        assert!(value.is_consistent());
    }

    #[test]
    fn hyphen_ranges_do_not_swallow_iso_dates() {
        let value = date(parse_date_expression("1877-1922"));
        assert_eq!(value.end.as_deref(), Some("1922"));
        let iso = date(parse_date_expression("1842-03-15"));
        assert_eq!(iso.end, None);
    }

    #[test]
    fn word_separators_are_understood() {
        let value = date(parse_date_expression("1877 to 1922"));
        assert_eq!(value.end.as_deref(), Some("1922"));
        let vi = date(parse_date_expression("1877 đến 1922"));
        assert_eq!(vi.end.as_deref(), Some("1922"));
    }

    #[test]
    fn inverted_range_is_refused() {
        assert_eq!(
            reason(parse_date_expression("1922–1877")),
            AmbiguousReason::InvertedRange
        );
    }

    #[test]
    fn impossible_calendar_dates_are_refused() {
        assert_eq!(
            reason(parse_date_expression("31/02/1842")),
            AmbiguousReason::InvalidDate
        );
        assert_eq!(
            reason(parse_date_expression("1842-13-01")),
            AmbiguousReason::Unparseable
        );
        assert!(matches!(
            parse_date_expression("29/02/1844"),
            DateParse::Date(_)
        ));
        assert_eq!(
            reason(parse_date_expression("29/02/1843")),
            AmbiguousReason::InvalidDate
        );
    }

    #[test]
    fn out_of_range_years_are_refused() {
        assert_eq!(
            reason(parse_date_expression("3000")),
            AmbiguousReason::OutOfRange
        );
        assert_eq!(
            reason(parse_date_expression("0999")),
            AmbiguousReason::OutOfRange
        );
    }

    #[test]
    fn a_lone_unknown_number_is_ambiguous_not_a_date() {
        assert_eq!(
            reason(parse_date_expression("197919")),
            AmbiguousReason::Unparseable
        );
    }

    #[test]
    fn prose_is_not_a_date() {
        assert_eq!(parse_date_expression("age 13"), DateParse::None);
        assert_eq!(parse_date_expression(""), DateParse::None);
        assert_eq!(parse_date_expression("died young"), DateParse::None);
    }

    #[test]
    fn leading_date_stops_before_trailing_prose() {
        let value = date(take_leading_date("1877 in Hà Nội"));
        assert_eq!(value.start, "1877");
        assert_eq!(value.precision, DatePrecision::Year);
        assert_eq!(
            date(take_leading_date("khoảng 1877")).qualifier,
            DateQualifier::Circa
        );
    }

    #[test]
    fn partial_iso_validation_matches_the_storage_contract() {
        assert!(is_iso_partial("1842"));
        assert!(is_iso_partial("1842-03"));
        assert!(is_iso_partial("1842-03-15"));
        assert!(!is_iso_partial("1842-13"));
        assert!(!is_iso_partial("1842-3-15"));
        assert!(!is_iso_partial("42"));
        assert!(!is_iso_partial("1842-02-30"));
        // Storage format only: plausibility is enforced before a value is stored.
        assert!(is_iso_partial("0999"));
    }
}
