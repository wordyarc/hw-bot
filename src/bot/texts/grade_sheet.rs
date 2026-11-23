use crate::app::{LateMark, ScoreCell};
use crate::domain::{DeductedFrom, Grading};

pub const BUTTON_GRADE_SHEET: &str = "📥 Ведомость";
pub const GRADE_SHEET_NO_STUDENTS: &str =
    "В этом предмете пока нет студентов, поэтому ведомость выгружать не из чего.";
pub const SHEET_LEGEND: &str = "6 (−2): в сумму пошло 6, за опоздание снято 2";
pub const SHEET_STUDENT: &str = "Студент";
pub const SHEET_PENDING_COLUMN: &str = "На проверке";
pub const SHEET_SCORE: &str = "Баллы";
pub const SHEET_MAX: &str = "из";
pub const SHEET_PASSED: &str = "зачёт";
pub const SHEET_PENDING: &str = "на проверке";
pub const SHEET_REJECTED: &str = "отклонено";
pub const SHEET_EMPTY: &str = "—";
pub const SHEET_FALLBACK_NAME: &str = "Группа";

const FROM_WORK: &str = "из балла";
const FROM_TOTAL: &str = "из итога";
const LATE: &str = "опоздание";
const MAKEUP: &str = "досдача";

fn mode(deducted_from: DeductedFrom) -> &'static str {
    match deducted_from {
        DeductedFrom::WorkScore => FROM_WORK,
        DeductedFrom::CourseTotal => FROM_TOTAL,
    }
}

pub fn sheet_title(subject: &str, group: &str, moment: &str) -> String {
    format!("{subject} · {group} · на {moment}")
}

pub fn sheet_accepted_header(assignments: usize) -> String {
    format!("Сдано\nиз {assignments}")
}

pub fn sheet_column_terms(grading: Grading, deducted_from: Option<DeductedFrom>) -> Vec<String> {
    let scale = match grading.max_score() {
        Some(max_score) => format!("из {max_score}"),
        None => SHEET_PASSED.to_owned(),
    };
    std::iter::once(scale)
        .chain(deducted_from.map(|value| format!("штраф {}", mode(value))))
        .collect()
}

pub fn sheet_column_header(
    grading: Grading,
    deducted_from: Option<DeductedFrom>,
    title: &str,
) -> String {
    let mut lines = sheet_column_terms(grading, deducted_from);
    lines.push(title.to_owned());
    lines.join("\n")
}

pub fn sheet_score_label(cell: &ScoreCell) -> Option<String> {
    let late = cell.late.map(|late| match late {
        LateMark::Removed { points, mode: None } => format!("−{points}"),
        LateMark::Removed {
            points,
            mode: Some(deducted_from),
        } => format!("−{points} {}", mode(deducted_from)),
        LateMark::NothingRemoved => LATE.to_owned(),
    });
    let makeup = cell.makeup.then(|| MAKEUP.to_owned());
    let scale = cell.frozen_scale.map(|scale| format!("из {scale}"));
    let notes: Vec<String> = [late, makeup, scale].into_iter().flatten().collect();
    if notes.is_empty() {
        return None;
    }

    Some(format!(
        "{} ({})",
        sheet_number(i64::from(cell.contribution)),
        notes.join(", ")
    ))
}

pub fn sheet_number(value: i64) -> String {
    if value < 0 {
        format!("−{}", value.unsigned_abs())
    } else {
        value.to_string()
    }
}

pub fn sheet_passed(makeup: bool) -> String {
    if makeup {
        format!("{SHEET_PASSED} ({MAKEUP})")
    } else {
        SHEET_PASSED.to_owned()
    }
}

pub fn sheet_file_name(subject: &str, date: &str) -> String {
    format!("Ведомость {subject} {date}.xlsx")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::DeductedFrom::{CourseTotal, WorkScore};

    fn score(contribution: i32) -> ScoreCell {
        ScoreCell {
            contribution,
            late: None,
            makeup: false,
            frozen_scale: None,
        }
    }

    fn removed(points: u16, mode: Option<DeductedFrom>) -> Option<LateMark> {
        Some(LateMark::Removed { points, mode })
    }

    #[test]
    fn a_plain_score_needs_no_label() {
        assert_eq!(sheet_score_label(&score(10)), None);
    }

    #[test]
    fn every_explanation_reads_as_the_spec_shows_it() {
        let cases = [
            (
                ScoreCell {
                    late: removed(2, None),
                    ..score(6)
                },
                "6 (−2)",
            ),
            (
                ScoreCell {
                    late: removed(1, None),
                    ..score(0)
                },
                "0 (−1)",
            ),
            (
                ScoreCell {
                    late: removed(2, None),
                    ..score(-1)
                },
                "−1 (−2)",
            ),
            (
                ScoreCell {
                    late: Some(LateMark::NothingRemoved),
                    ..score(0)
                },
                "0 (опоздание)",
            ),
            (
                ScoreCell {
                    late: removed(2, Some(WorkScore)),
                    ..score(6)
                },
                "6 (−2 из балла)",
            ),
            (
                ScoreCell {
                    late: removed(2, Some(CourseTotal)),
                    ..score(6)
                },
                "6 (−2 из итога)",
            ),
            (
                ScoreCell {
                    makeup: true,
                    ..score(6)
                },
                "6 (досдача)",
            ),
            (
                ScoreCell {
                    frozen_scale: Some(10),
                    ..score(8)
                },
                "8 (из 10)",
            ),
            (
                ScoreCell {
                    late: removed(2, None),
                    frozen_scale: Some(10),
                    ..score(6)
                },
                "6 (−2, из 10)",
            ),
            (
                ScoreCell {
                    late: removed(2, Some(WorkScore)),
                    frozen_scale: Some(10),
                    ..score(6)
                },
                "6 (−2 из балла, из 10)",
            ),
        ];

        for (cell, expected) in cases {
            assert_eq!(sheet_score_label(&cell).as_deref(), Some(expected));
        }
    }

    #[test]
    fn a_pass_names_its_makeup() {
        assert_eq!(sheet_passed(false), "зачёт");
        assert_eq!(sheet_passed(true), "зачёт (досдача)");
    }

    #[test]
    fn a_column_header_names_the_scale_the_penalty_mode_and_the_title() {
        let ten = Grading::scored(10).unwrap();

        assert_eq!(
            sheet_column_header(ten, Some(CourseTotal), "ДЗ 1"),
            "из 10\nштраф из итога\nДЗ 1"
        );
        assert_eq!(
            sheet_column_header(ten, Some(WorkScore), "ДЗ 2"),
            "из 10\nштраф из балла\nДЗ 2"
        );
        assert_eq!(sheet_column_header(ten, None, "ДЗ 3"), "из 10\nДЗ 3");
        assert_eq!(
            sheet_column_header(Grading::PassFail, None, "Лаба"),
            "зачёт\nЛаба"
        );
    }

    #[test]
    fn a_negative_number_uses_the_typographic_minus() {
        assert_eq!(sheet_number(-4), "−4");
        assert_eq!(sheet_number(4), "4");
    }
}
