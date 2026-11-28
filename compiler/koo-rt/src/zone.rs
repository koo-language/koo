//! Time zones (Section 10.6): the offset from UTC that a zone of the IANA database has
//! at an instant, and the instant that a time on a zone's clocks is.
//!
//! The zones are carried in the program, in `tzdata.bin` (`tz/make.py` writes it from a
//! named release of the database), so that a program says the same of a zone on every
//! machine, a container with nothing else in it among them. Each zone is a TZif file
//! (RFC 8536), read where it lies: the instants at which its offset changes, in order,
//! and after the last of them the rule its footer gives in the form of POSIX's `TZ`.
//! Nothing is read ahead or kept, so a zone costs nothing until it is asked about, and
//! the threads of a program ask at once without a lock.

use crate::{Header, bytes, new_str};

static DATA: &[u8] = include_bytes!("tzdata.bin");

fn u16_at(b: &[u8], at: usize) -> usize {
    u16::from_be_bytes([b[at], b[at + 1]]) as usize
}

fn u32_at(b: &[u8], at: usize) -> usize {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as usize
}

fn i32_at(b: &[u8], at: usize) -> i64 {
    i32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as i64
}

fn i64_at(b: &[u8], at: usize) -> i64 {
    let mut word = [0u8; 8];
    word.copy_from_slice(&b[at..at + 8]);
    i64::from_be_bytes(word)
}

/// The TZif file of a zone, by its name: `Europe/Paris`, `UTC`.
fn file(name: &[u8]) -> Option<&'static [u8]> {
    let names = u16_at(DATA, 16);
    let files = u16_at(DATA, 18);
    let entries = 20;
    let starts = entries + names * 6;
    let text = starts + (files + 1) * 4;
    // The names end where the first file begins, which is not written down: it is the
    // end of the last name.
    let last = entries + (names - 1) * 6;
    let data = text + u16_at(DATA, last) + DATA[last + 2] as usize;
    let name_of = |i: usize| {
        let at = text + u16_at(DATA, entries + i * 6);
        &DATA[at..at + DATA[entries + i * 6 + 2] as usize]
    };
    let (mut low, mut high) = (0, names);
    while low < high {
        let middle = (low + high) / 2;
        match name_of(middle).cmp(name) {
            core::cmp::Ordering::Less => low = middle + 1,
            core::cmp::Ordering::Greater => high = middle,
            core::cmp::Ordering::Equal => {
                let number = u16_at(DATA, entries + middle * 6 + 4);
                let begin = data + u32_at(DATA, starts + number * 4);
                let end = data + u32_at(DATA, starts + (number + 1) * 4);
                return Some(&DATA[begin..end]);
            }
        }
    }
    None
}

/// The parts of a TZif file of version 2 or later that are read: those of the second
/// header, whose instants are 64 bits.
struct Zone {
    data: &'static [u8],
    count: usize,
    times: usize,
    kinds: usize,
    types: usize,
    chars: usize,
    footer: &'static [u8],
}

impl Zone {
    fn of(data: &'static [u8]) -> Option<Zone> {
        if data.len() < 44 || &data[..4] != b"TZif" || data[4] < b'2' {
            return None;
        }
        let counts = |at: usize| {
            (
                u32_at(data, at + 20),
                u32_at(data, at + 24),
                u32_at(data, at + 28),
                u32_at(data, at + 32),
                u32_at(data, at + 36),
                u32_at(data, at + 40),
            )
        };
        let (ut, std, leap, time, kind, chars) = counts(0);
        let second = 44 + time * 5 + kind * 6 + chars + leap * 8 + std + ut;
        if data.len() < second + 44 || &data[second..second + 4] != b"TZif" {
            return None;
        }
        let (ut, std, leap, time, kind, chars) = counts(second);
        let times = second + 44;
        let kinds = times + time * 8;
        let types = kinds + time;
        let text = types + kind * 6;
        let end = text + chars + leap * 12 + std + ut;
        if data.len() < end || kind == 0 {
            return None;
        }
        // The footer is a line between two newlines.
        let rest = &data[end..];
        let footer = match rest.iter().skip(1).position(|&b| b == b'\n') {
            Some(at) if rest.first() == Some(&b'\n') => &rest[1..at + 1],
            _ => &[][..],
        };
        Some(Zone {
            data,
            count: time,
            times,
            kinds,
            types,
            chars: text,
            footer,
        })
    }

    fn time(&self, i: usize) -> i64 {
        i64_at(self.data, self.times + i * 8)
    }

    /// The offset and the abbreviation of a local time type.
    fn kind(&self, k: usize) -> (i64, &'static [u8]) {
        let at = self.types + k * 6;
        let offset = i32_at(self.data, at);
        let from = self.chars + self.data[at + 5] as usize;
        let len = self.data[from..].iter().position(|&b| b == 0).unwrap_or(0);
        (offset, &self.data[from..from + len])
    }

    /// The offset from UTC, in seconds, at an instant, and the abbreviation for it.
    fn at(&self, t: i64) -> (i64, &'static [u8]) {
        if self.count == 0 || t >= self.time(self.count - 1) {
            if let Some(rule) = Rule::of(self.footer) {
                return rule.at(t);
            }
            if self.count == 0 {
                return self.kind(0);
            }
            return self.kind(self.data[self.kinds + self.count - 1] as usize);
        }
        if t < self.time(0) {
            return self.kind(0);
        }
        // The last change at or before the instant.
        let (mut low, mut high) = (0, self.count - 1);
        while low < high {
            let middle = (low + high).div_ceil(2);
            if self.time(middle) <= t {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        self.kind(self.data[self.kinds + low] as usize)
    }
}

/// The day of a year on which a rule of POSIX's `TZ` changes the clocks.
#[derive(Clone, Copy)]
enum Day {
    /// `Jn`: the nth day of the year, from 1, the 29th of February never counted.
    Julian(i64),
    /// `n`: the nth day of the year, from 0, the 29th of February counted.
    Ordinal(i64),
    /// `Mm.w.d`: the dth day of the week (0 is Sunday) of the wth week of month m, the
    /// fifth being the last.
    Month(i64, i64, i64),
}

/// A footer: a zone's standard time, and its summer time and when it begins and ends,
/// if it has one. The offsets are seconds east of Greenwich, as TZif's are, which is the
/// opposite of how `TZ` writes them.
struct Rule {
    standard: (i64, &'static [u8]),
    summer: Option<Summer>,
}

/// A rule's summer time: its offset and name, and the day and the time of day at which
/// it begins, and at which it ends.
#[derive(Clone, Copy)]
struct Summer {
    offset: i64,
    name: &'static [u8],
    begins: (Day, i64),
    ends: (Day, i64),
}

struct Reader {
    text: &'static [u8],
    at: usize,
}

impl Reader {
    fn peek(&self) -> Option<u8> {
        self.text.get(self.at).copied()
    }

    fn name(&mut self) -> Option<&'static [u8]> {
        if self.peek() == Some(b'<') {
            let from = self.at + 1;
            let len = self.text[from..].iter().position(|&b| b == b'>')?;
            self.at = from + len + 1;
            return Some(&self.text[from..from + len]);
        }
        let from = self.at;
        while self.peek().is_some_and(|b| b.is_ascii_alphabetic()) {
            self.at += 1;
        }
        (self.at - from >= 3).then(|| &self.text[from..self.at])
    }

    fn number(&mut self) -> Option<i64> {
        let from = self.at;
        let mut n: i64 = 0;
        while let Some(b) = self.peek().filter(u8::is_ascii_digit) {
            n = n.checked_mul(10)?.checked_add((b - b'0') as i64)?;
            self.at += 1;
        }
        (self.at > from).then_some(n)
    }

    /// `[+-]hh[:mm[:ss]]`, in seconds.
    fn clock(&mut self) -> Option<i64> {
        let sign = match self.peek() {
            Some(b'-') => {
                self.at += 1;
                -1
            }
            Some(b'+') => {
                self.at += 1;
                1
            }
            _ => 1,
        };
        let mut seconds = self.number()? * 3600;
        for size in [60, 1] {
            if self.peek() != Some(b':') {
                break;
            }
            self.at += 1;
            seconds += self.number()? * size;
        }
        Some(sign * seconds)
    }

    fn day(&mut self) -> Option<(Day, i64)> {
        let day = match self.peek()? {
            b'J' => {
                self.at += 1;
                Day::Julian(self.number()?)
            }
            b'M' => {
                self.at += 1;
                let month = self.number()?;
                (self.peek() == Some(b'.')).then_some(())?;
                self.at += 1;
                let week = self.number()?;
                (self.peek() == Some(b'.')).then_some(())?;
                self.at += 1;
                Day::Month(month, week, self.number()?)
            }
            _ => Day::Ordinal(self.number()?),
        };
        let time = if self.peek() == Some(b'/') {
            self.at += 1;
            self.clock()?
        } else {
            7200
        };
        Some((day, time))
    }
}

impl Rule {
    fn of(text: &'static [u8]) -> Option<Rule> {
        if text.is_empty() {
            return None;
        }
        let mut r = Reader { text, at: 0 };
        let name = r.name()?;
        let standard = -r.clock()?;
        if r.peek().is_none() {
            return Some(Rule {
                standard: (standard, name),
                summer: None,
            });
        }
        let summer_name = r.name()?;
        let summer = if r.peek().is_some_and(|b| b != b',') {
            -r.clock()?
        } else {
            standard + 3600
        };
        // Without the days, they are those of the United States.
        let (begins, ends) = if r.peek() == Some(b',') {
            r.at += 1;
            let begins = r.day()?;
            (r.peek() == Some(b',')).then_some(())?;
            r.at += 1;
            (begins, r.day()?)
        } else {
            ((Day::Month(3, 2, 0), 7200), (Day::Month(11, 1, 0), 7200))
        };
        if r.peek().is_some() {
            return None;
        }
        Some(Rule {
            standard: (standard, name),
            summer: Some(Summer {
                offset: summer,
                name: summer_name,
                begins,
                ends,
            }),
        })
    }

    fn at(&self, t: i64) -> (i64, &'static [u8]) {
        let Some(Summer {
            offset: summer,
            name: summer_name,
            begins: (begins, begins_at),
            ends: (ends, ends_at),
        }) = self.summer
        else {
            return self.standard;
        };
        let year = civil((t + self.standard.0).div_euclid(86400)).0;
        // The change to summer time is at a time of standard time, and the change back
        // at a time of summer time.
        let start = day_of(year, begins) * 86400 + begins_at - self.standard.0;
        let end = day_of(year, ends) * 86400 + ends_at - summer;
        let in_summer = if start < end {
            start <= t && t < end
        } else {
            !(end <= t && t < start)
        };
        if in_summer {
            (summer, summer_name)
        } else {
            self.standard
        }
    }
}

fn leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// The days from the first of January 1970 to a date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let shifted = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146097 + day_of_era - 719468
}

/// The year, the month and the day of a day counted from the first of January 1970.
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let day_of_era = z - era * 146097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 { shifted + 3 } else { shifted - 9 };
    (year_of_era + era * 400 + (month <= 2) as i64, month, day)
}

fn month_days(year: i64, month: i64) -> i64 {
    match month {
        2 if leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The day, counted from the first of January 1970, on which a rule's day falls in a
/// year.
fn day_of(year: i64, day: Day) -> i64 {
    let first = days_from_civil(year, 1, 1);
    match day {
        Day::Julian(n) => first + n - 1 + (leap(year) && n >= 60) as i64,
        Day::Ordinal(n) => first + n,
        Day::Month(month, week, weekday) => {
            let month = month.clamp(1, 12);
            let start = days_from_civil(year, month, 1);
            // The first of January 1970 was a Thursday, the fourth day from Sunday.
            let first_weekday = (start + 4).rem_euclid(7);
            let mut day = start + (weekday - first_weekday).rem_euclid(7) + (week.clamp(1, 5) - 1) * 7;
            while day >= start + month_days(year, month) {
                day -= 7;
            }
            day
        }
    }
}

fn zone(name: *const Header) -> Option<Zone> {
    Zone::of(file(unsafe { bytes(name) })?)
}

/// Whether the database has a zone of this name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_zone_known(name: *const Header) -> bool {
    zone(name).is_some()
}

/// The offset from UTC, in seconds, that a zone has at an instant, given in seconds from
/// the beginning of 1970.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_zone_offset(name: *const Header, t: i64) -> i64 {
    zone(name).map_or(0, |zone| zone.at(t).0)
}

/// What the zone's clocks are called at an instant: `CET`, `CEST`, `+0530`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_zone_abbreviation(name: *const Header, t: i64) -> *mut Header {
    let said = zone(name).map_or(&b""[..], |zone| zone.at(t).1);
    unsafe { new_str(said) }
}

/// The instant, in seconds from the beginning of 1970, at which a zone's clocks show a
/// time, given as the seconds it would be from the beginning of 1970 if it were in UTC.
/// Where the clocks show it twice, when they are put back, it is the first of the two;
/// where they never show it, when they are put forward, it is the time as the offset
/// before the change has it, which the clocks show as that much later.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_zone_local(name: *const Header, local: i64) -> i64 {
    let Some(zone) = zone(name) else {
        return local;
    };
    // The offsets around the time, which a change of the clocks lies between.
    let mut found: Option<i64> = None;
    let mut earliest = i64::MAX;
    for around in [-172_800, 0, 172_800] {
        let offset = zone.at(local.saturating_add(around)).0;
        let t = local - offset;
        earliest = earliest.min(t);
        if zone.at(t).0 == offset {
            found = Some(found.map_or(t, |other: i64| other.min(t)));
        }
    }
    match found {
        Some(t) => t,
        None => local - zone.at(earliest).0,
    }
}

/// The release of the database the zones are from: `2026f`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_zone_release() -> *mut Header {
    let release = &DATA[8..16];
    let len = release.iter().position(|&b| b == 0).unwrap_or(8);
    unsafe { new_str(&release[..len]) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zones_are_found_by_name() {
        assert!(file(b"Europe/Paris").is_some());
        assert!(file(b"UTC").is_some());
        assert!(file(b"Africa/Abidjan").is_some());
        assert!(file(b"Zulu").is_some());
        assert!(file(b"Europe/Pariss").is_none());
        assert!(file(b"").is_none());
    }

    #[test]
    fn a_footer_is_read() {
        let rule = Rule::of(b"CET-1CEST,M3.5.0,M10.5.0/3").unwrap();
        assert_eq!(rule.standard.0, 3600);
        // 2030-07-01T00:00:00Z is summer; 2030-01-01T00:00:00Z is not.
        assert_eq!(rule.at(1909008000).0, 7200);
        assert_eq!(rule.at(1893456000).0, 3600);
        assert!(Rule::of(b"<+0530>-5:30").is_some_and(|rule| rule.standard.0 == 19800));
    }
}
