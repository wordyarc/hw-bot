use std::collections::HashSet;

use chrono::{DateTime, Utc};
use rust_xlsxwriter::{Color, Format, FormatAlign, FormatBorder, Workbook, Worksheet, XlsxError};

use super::files::GeneratedFile;
use super::texts;
use super::timezone::BotTimezone;
use crate::app::{
    GradeSheet, GradeSheetCell, GradeSheetColumn, GradeSheetGroup, GradeSheetStudent,
};

const TITLE_ROW: u32 = 0;
const LEGEND_ROW: u32 = 1;
const HEADER_ROW: u32 = 2;
const FIRST_STUDENT_ROW: u32 = 3;

const STUDENT_COLUMN: u16 = 0;
const ACCEPTED_COLUMN: u16 = 1;
const PENDING_COLUMN: u16 = 2;
const SCORE_COLUMN: u16 = 3;
const MAX_COLUMN: u16 = 4;
const FIRST_ASSIGNMENT_COLUMN: u16 = 5;

const SUM_FORMAT: &str = "0;\"−\"0;0";
const FILTER_BUTTON_WIDTH: usize = 3;
const STUDENT_MAX_WIDTH: usize = 32;
const TITLE_MIN_WIDTH: usize = 14;
const HEADER_LINE_HEIGHT: f64 = 15.0;
const HEADER_MAX_LINES: usize = 6;

const SHEET_NAME_MAX_UTF16: usize = 31;
const SHEET_NAME_FORBIDDEN: [char; 7] = ['/', '\\', ':', '*', '?', '[', ']'];
const RESERVED_SHEET_NAME: &str = "history";
const FILE_NAME_FORBIDDEN: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
const SUBJECT_IN_FILE_NAME_MAX: usize = 100;

const MUTED: Color = Color::RGB(0x6B6B6B);
const HEADER_FILL: Color = Color::RGB(0xEDEDED);
const ALL_ACCEPTED_FILL: Color = Color::RGB(0xC6EFCE);

struct Styles {
    title: Format,
    legend: Format,
    header: Format,
    student: Format,
    number: Format,
    sum: Format,
    all_accepted: Format,
    text: Format,
    muted: Format,
}

impl Styles {
    fn new() -> Self {
        let centered = Format::new().set_align(FormatAlign::Center);
        Self {
            title: Format::new().set_bold().set_font_size(13),
            legend: Format::new().set_font_color(MUTED),
            header: Format::new()
                .set_bold()
                .set_text_wrap()
                .set_align(FormatAlign::Center)
                .set_align(FormatAlign::Bottom)
                .set_background_color(HEADER_FILL)
                .set_border_bottom(FormatBorder::Thin),
            student: Format::new().set_text_wrap(),
            number: centered.clone(),
            sum: centered.clone().set_num_format(SUM_FORMAT),
            all_accepted: centered
                .clone()
                .set_bold()
                .set_background_color(ALL_ACCEPTED_FILL),
            text: centered.clone(),
            muted: centered.set_font_color(MUTED),
        }
    }
}

pub(super) fn build(
    sheet: &GradeSheet,
    now: DateTime<Utc>,
    timezone: &BotTimezone,
) -> Result<GeneratedFile, XlsxError> {
    let subject = sheet.subject.name().as_str();
    let moment = timezone.format_moment(now);
    let date = timezone.local_date(now).format("%d.%m.%Y").to_string();
    let styles = Styles::new();
    let mut workbook = Workbook::new();
    let names = sheet_names(sheet.groups.iter().map(|group| group.code.as_str()));
    for (group, name) in sheet.groups.iter().zip(names) {
        let worksheet = workbook.add_worksheet().set_name(name)?;
        let title = texts::sheet_title(subject, &group.code, &moment);
        write_group(worksheet, &styles, &title, group)?;
    }

    Ok(GeneratedFile {
        filename: file_name(subject, &date),
        content: workbook.save_to_buffer()?,
    })
}

fn write_group(
    worksheet: &mut Worksheet,
    styles: &Styles,
    title: &str,
    group: &GradeSheetGroup,
) -> Result<(), XlsxError> {
    worksheet.write_string_with_format(TITLE_ROW, STUDENT_COLUMN, title, &styles.title)?;
    worksheet.write_string_with_format(
        LEGEND_ROW,
        STUDENT_COLUMN,
        texts::SHEET_LEGEND,
        &styles.legend,
    )?;
    let fixed = [
        (STUDENT_COLUMN, texts::SHEET_STUDENT.to_owned()),
        (
            ACCEPTED_COLUMN,
            texts::sheet_accepted_header(group.columns.len()),
        ),
        (PENDING_COLUMN, texts::SHEET_PENDING_COLUMN.to_owned()),
        (SCORE_COLUMN, texts::SHEET_SCORE.to_owned()),
        (MAX_COLUMN, texts::SHEET_MAX.to_owned()),
    ];
    for (column, header) in fixed {
        worksheet.write_string_with_format(HEADER_ROW, column, header, &styles.header)?;
    }
    for (column, assignment) in (FIRST_ASSIGNMENT_COLUMN..).zip(&group.columns) {
        let header = texts::sheet_column_header(
            assignment.grading,
            assignment.deducted_from,
            &assignment.title,
        );
        worksheet.write_string_with_format(HEADER_ROW, column, header, &styles.header)?;
    }
    for (row, student) in (FIRST_STUDENT_ROW..).zip(&group.students) {
        write_student(worksheet, styles, row, student, group.columns.len())?;
    }

    let widths = column_widths(group);
    for (column, width) in (STUDENT_COLUMN..).zip(&widths) {
        worksheet.set_column_width(column, *width as f64)?;
    }
    worksheet.set_row_height(HEADER_ROW, header_height(group, &widths))?;
    let last_row = HEADER_ROW + group.students.len() as u32;
    let last_column = FIRST_ASSIGNMENT_COLUMN + group.columns.len() as u16 - 1;
    worksheet.autofilter(HEADER_ROW, STUDENT_COLUMN, last_row, last_column)?;
    worksheet.set_freeze_panes(FIRST_STUDENT_ROW, ACCEPTED_COLUMN)?;
    Ok(())
}

fn write_student(
    worksheet: &mut Worksheet,
    styles: &Styles,
    row: u32,
    student: &GradeSheetStudent,
    assignments: usize,
) -> Result<(), XlsxError> {
    worksheet.write_string_with_format(
        row,
        STUDENT_COLUMN,
        student.display_name.as_str(),
        &styles.student,
    )?;
    let accepted = if assignments > 0 && student.accepted as usize == assignments {
        &styles.all_accepted
    } else {
        &styles.number
    };
    worksheet.write_number_with_format(row, ACCEPTED_COLUMN, student.accepted, accepted)?;
    if student.pending > 0 {
        worksheet.write_number_with_format(row, PENDING_COLUMN, student.pending, &styles.number)?;
    }
    worksheet.write_number_with_format(row, SCORE_COLUMN, student.total as f64, &styles.sum)?;
    worksheet.write_number_with_format(row, MAX_COLUMN, student.max_total, &styles.number)?;
    for (column, cell) in (FIRST_ASSIGNMENT_COLUMN..).zip(&student.cells) {
        write_cell(worksheet, styles, row, column, cell)?;
    }
    Ok(())
}

fn write_cell(
    worksheet: &mut Worksheet,
    styles: &Styles,
    row: u32,
    column: u16,
    cell: &GradeSheetCell,
) -> Result<(), XlsxError> {
    match cell {
        GradeSheetCell::Score(score) => match texts::sheet_score_label(score) {
            Some(label) => {
                worksheet.write_string_with_format(row, column, label, &styles.text)?;
            }
            None => {
                worksheet.write_number_with_format(
                    row,
                    column,
                    score.contribution,
                    &styles.number,
                )?;
            }
        },
        GradeSheetCell::Passed { .. } => {
            worksheet.write_string_with_format(row, column, cell_label(cell), &styles.text)?;
        }
        GradeSheetCell::Pending | GradeSheetCell::Rejected | GradeSheetCell::Empty => {
            worksheet.write_string_with_format(row, column, cell_label(cell), &styles.muted)?;
        }
    }
    Ok(())
}

fn cell_label(cell: &GradeSheetCell) -> String {
    match cell {
        GradeSheetCell::Score(score) => texts::sheet_score_label(score)
            .unwrap_or_else(|| texts::sheet_number(i64::from(score.contribution))),
        GradeSheetCell::Passed { makeup } => texts::sheet_passed(*makeup),
        GradeSheetCell::Pending => texts::SHEET_PENDING.to_owned(),
        GradeSheetCell::Rejected => texts::SHEET_REJECTED.to_owned(),
        GradeSheetCell::Empty => texts::SHEET_EMPTY.to_owned(),
    }
}

fn chars(value: &str) -> usize {
    value.chars().count()
}

fn longest_line(value: &str) -> usize {
    value.split('\n').map(chars).max().unwrap_or(0)
}

fn column_widths(group: &GradeSheetGroup) -> Vec<usize> {
    let student = group
        .students
        .iter()
        .map(|student| chars(&student.display_name))
        .chain([chars(texts::SHEET_STUDENT)])
        .max()
        .unwrap_or(0);
    let accepted = longest_line(&texts::sheet_accepted_header(group.columns.len()));
    let pending = chars(texts::SHEET_PENDING_COLUMN);
    let score = group
        .students
        .iter()
        .map(|student| chars(&texts::sheet_number(student.total)))
        .chain([chars(texts::SHEET_SCORE)])
        .max()
        .unwrap_or(0);
    let max = group
        .students
        .iter()
        .map(|student| chars(&student.max_total.to_string()))
        .chain([chars(texts::SHEET_MAX)])
        .max()
        .unwrap_or(0);

    [
        (student + FILTER_BUTTON_WIDTH).min(STUDENT_MAX_WIDTH),
        accepted + FILTER_BUTTON_WIDTH,
        pending + FILTER_BUTTON_WIDTH,
        score + FILTER_BUTTON_WIDTH,
        max + FILTER_BUTTON_WIDTH,
    ]
    .into_iter()
    .chain(
        group
            .columns
            .iter()
            .enumerate()
            .map(|(index, column)| assignment_width(group, index, column)),
    )
    .collect()
}

fn assignment_width(group: &GradeSheetGroup, index: usize, column: &GradeSheetColumn) -> usize {
    let values = group
        .students
        .iter()
        .filter_map(|student| student.cells.get(index))
        .map(|cell| chars(&cell_label(cell)));
    let terms = texts::sheet_column_terms(column.grading, column.deducted_from);
    let terms = terms.iter().map(|line| chars(line));
    let title = chars(&column.title).min(TITLE_MIN_WIDTH);

    values.chain(terms).chain([title]).max().unwrap_or(0) + FILTER_BUTTON_WIDTH
}

fn header_height(group: &GradeSheetGroup, widths: &[usize]) -> f64 {
    let assignment_lines = group
        .columns
        .iter()
        .zip(&widths[FIRST_ASSIGNMENT_COLUMN as usize..])
        .map(|(column, width)| {
            let terms = texts::sheet_column_terms(column.grading, column.deducted_from).len();
            let text_width = width.saturating_sub(FILTER_BUTTON_WIDTH).max(1);
            terms + chars(&column.title).div_ceil(text_width)
        });
    let lines = assignment_lines
        .chain([texts::sheet_accepted_header(group.columns.len())
            .split('\n')
            .count()])
        .max()
        .unwrap_or(1)
        .min(HEADER_MAX_LINES);

    lines as f64 * HEADER_LINE_HEIGHT
}

fn sheet_names<'a>(codes: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut used = HashSet::from([RESERVED_SHEET_NAME.to_owned()]);
    codes
        .map(|code| {
            let base = clean_sheet_name(code);
            let mut name = fit_sheet_name(&base, "");
            let mut number = 2;
            while !used.insert(name.to_lowercase()) {
                name = fit_sheet_name(&base, &format!(" ({number})"));
                number += 1;
            }
            name
        })
        .collect()
}

fn clean_sheet_name(code: &str) -> String {
    let cleaned: String = code
        .chars()
        .map(|character| {
            if character.is_control() || SHEET_NAME_FORBIDDEN.contains(&character) {
                '_'
            } else {
                character
            }
        })
        .collect();
    let trimmed = trim_sheet_name(&cleaned);
    if trimmed.is_empty() {
        texts::SHEET_FALLBACK_NAME.to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn trim_sheet_name(name: &str) -> &str {
    name.trim_matches(|character: char| character == '\'' || character.is_whitespace())
}

fn fit_sheet_name(base: &str, suffix: &str) -> String {
    let limit = SHEET_NAME_MAX_UTF16 - suffix.encode_utf16().count();
    let mut used = 0;
    let prefix: String = base
        .chars()
        .take_while(|character| {
            used += character.len_utf16();
            used <= limit
        })
        .collect();

    format!("{}{suffix}", trim_sheet_name(&prefix))
}

fn file_name(subject: &str, date: &str) -> String {
    let subject: String = subject
        .chars()
        .filter(|character| !character.is_control() && !FILE_NAME_FORBIDDEN.contains(character))
        .take(SUBJECT_IN_FILE_NAME_MAX)
        .collect();

    texts::sheet_file_name(&subject, date)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use calamine::{Data, Reader, Xlsx};
    use chrono::TimeZone;
    use chrono_tz::Asia::Vladivostok;

    use super::*;
    use crate::domain::DeductedFrom::{CourseTotal, WorkScore};
    use crate::domain::{
        AssignmentId, DeductedFrom, FILE_NAME_MAX_LEN, Grade, Grading, LatePenalty, StudentGroupId,
        UserId,
    };
    use crate::read_models::{AcceptedWork, GradeSheetRow, SubmissionStates};
    use crate::test_support::subject;

    fn column(title: &str, max: Option<u16>, mode: Option<DeductedFrom>) -> GradeSheetColumn {
        GradeSheetColumn {
            assignment_id: AssignmentId::new(),
            title: title.to_owned(),
            grading: max.map_or(Grading::PassFail, |max| Grading::scored(max).unwrap()),
            deducted_from: mode,
        }
    }

    fn graded(raw: u16, max: u16, late: Option<(u16, DeductedFrom)>) -> SubmissionStates {
        let late = late.and_then(|(points, mode)| LatePenalty::restore(1, points, mode));
        SubmissionStates {
            accepted: Some(AcceptedWork {
                grade: Some(Grade::new(raw, max, late)),
                makeup: false,
            }),
            ..SubmissionStates::default()
        }
    }

    fn passed() -> SubmissionStates {
        SubmissionStates {
            accepted: Some(AcceptedWork {
                grade: None,
                makeup: false,
            }),
            ..SubmissionStates::default()
        }
    }

    fn pending() -> SubmissionStates {
        SubmissionStates {
            pending: true,
            ..SubmissionStates::default()
        }
    }

    fn rejected() -> SubmissionStates {
        SubmissionStates {
            rejected: true,
            ..SubmissionStates::default()
        }
    }

    fn sheet_of(
        students: &[(&str, Vec<SubmissionStates>)],
        columns: &[GradeSheetColumn],
    ) -> GradeSheet {
        let group = StudentGroupId::new();
        let rows = students
            .iter()
            .flat_map(|(name, states)| {
                let id = UserId::new();
                columns
                    .iter()
                    .zip(states)
                    .map(move |(column, state)| GradeSheetRow {
                        group_id: group,
                        group_code: "Б9123".to_owned(),
                        student_id: id,
                        display_name: (*name).to_owned(),
                        assignment: Some((column.clone(), *state)),
                    })
            })
            .collect();
        GradeSheet::assemble(subject("Алгоритмы"), rows).unwrap()
    }

    fn sample() -> GradeSheet {
        let columns = [
            column("ДЗ 1", Some(10), Some(CourseTotal)),
            column("ДЗ 2", Some(10), None),
            column("Лаба", None, None),
        ];
        sheet_of(
            &[
                (
                    "Иванов Иван",
                    vec![
                        graded(8, 10, Some((2, CourseTotal))),
                        graded(10, 10, None),
                        passed(),
                    ],
                ),
                (
                    "Петрова Анна",
                    vec![graded(1, 10, Some((3, CourseTotal))), pending(), rejected()],
                ),
            ],
            &columns,
        )
    }

    fn built(sheet: &GradeSheet) -> GeneratedFile {
        let now = Utc.with_ymd_and_hms(2026, 11, 23, 4, 5, 0).unwrap();
        build(sheet, now, &BotTimezone::new(Vladivostok)).unwrap()
    }

    fn workbook(file: &GeneratedFile) -> Xlsx<Cursor<Vec<u8>>> {
        Xlsx::new(Cursor::new(file.content.clone())).unwrap()
    }

    fn value(range: &calamine::Range<Data>, row: u32, column: u32) -> Data {
        range
            .get_value((row, column))
            .cloned()
            .unwrap_or(Data::Empty)
    }

    fn text(value: &str) -> Data {
        Data::String(value.to_owned())
    }

    #[test]
    fn the_head_names_the_subject_the_group_and_the_moment_in_the_local_zone() {
        let mut book = workbook(&built(&sample()));
        let range = book.worksheet_range("Б9123").unwrap();

        assert_eq!(
            value(&range, TITLE_ROW, 0),
            text("Алгоритмы · Б9123 · на 23.11.2026 14:05")
        );
        assert_eq!(value(&range, LEGEND_ROW, 0), text(texts::SHEET_LEGEND));
    }

    #[test]
    fn the_columns_go_student_accepted_pending_score_maximum_then_assignments() {
        let mut book = workbook(&built(&sample()));
        let range = book.worksheet_range("Б9123").unwrap();

        let headers: Vec<Data> = (0..8).map(|col| value(&range, HEADER_ROW, col)).collect();

        assert_eq!(
            headers,
            [
                text("Студент"),
                text("Сдано\nиз 3"),
                text("На проверке"),
                text("Баллы"),
                text("из"),
                text("из 10\nштраф из итога\nДЗ 1"),
                text("из 10\nДЗ 2"),
                text("зачёт\nЛаба"),
            ]
        );
    }

    #[test]
    fn totals_are_numbers_so_they_sort_numerically() {
        let mut book = workbook(&built(&sample()));
        let range = book.worksheet_range("Б9123").unwrap();

        let row: Vec<Data> = (1..5)
            .map(|col| value(&range, FIRST_STUDENT_ROW, col))
            .collect();

        assert_eq!(
            row,
            [
                Data::Float(3.0),
                Data::Empty,
                Data::Float(16.0),
                Data::Float(20.0),
            ]
        );
    }

    #[test]
    fn a_negative_sum_stays_a_number_and_a_pending_count_is_shown() {
        let mut book = workbook(&built(&sample()));
        let range = book.worksheet_range("Б9123").unwrap();

        assert_eq!(value(&range, FIRST_STUDENT_ROW + 1, 2), Data::Float(1.0));
        assert_eq!(value(&range, FIRST_STUDENT_ROW + 1, 3), Data::Float(-2.0));
    }

    #[test]
    fn a_cell_with_an_explanation_is_text_and_a_plain_score_is_a_number() {
        let mut book = workbook(&built(&sample()));
        let range = book.worksheet_range("Б9123").unwrap();

        assert_eq!(value(&range, FIRST_STUDENT_ROW, 5), text("6 (−2)"));
        assert_eq!(value(&range, FIRST_STUDENT_ROW, 6), Data::Float(10.0));
        assert_eq!(value(&range, FIRST_STUDENT_ROW, 7), text("зачёт"));
        assert_eq!(value(&range, FIRST_STUDENT_ROW + 1, 5), text("−2 (−3)"));
        assert_eq!(value(&range, FIRST_STUDENT_ROW + 1, 6), text("на проверке"));
        assert_eq!(value(&range, FIRST_STUDENT_ROW + 1, 7), text("отклонено"));
    }

    #[test]
    fn every_group_is_its_own_sheet_in_order() {
        let mut sheet = sample();
        let mut second = sheet.groups[0].clone();
        second.id = StudentGroupId::new();
        second.code = "Б9124".to_owned();
        sheet.groups.push(second);

        let book = workbook(&built(&sheet));

        assert_eq!(book.sheet_names(), ["Б9123", "Б9124"]);
    }

    #[test]
    fn a_name_that_looks_like_a_formula_stays_text_and_the_book_has_no_formulas() {
        let name = "=HYPERLINK(\"https://example.test\",\"x\")";
        let columns = [column("=1+1", Some(10), None)];
        let sheet = sheet_of(&[(name, vec![graded(5, 10, None)])], &columns);

        let mut book = workbook(&built(&sheet));

        let range = book.worksheet_range("Б9123").unwrap();
        assert_eq!(value(&range, FIRST_STUDENT_ROW, 0), text(name));
        assert_eq!(value(&range, HEADER_ROW, 5), text("из 10\n=1+1"));
        let formulas = book.worksheet_formula("Б9123").unwrap();
        assert!(
            formulas
                .used_cells()
                .all(|(_, _, formula)| formula.is_empty())
        );
    }

    #[test]
    fn a_column_is_wide_enough_for_its_longest_label() {
        let columns = [column("ДЗ", Some(20), Some(CourseTotal))];
        let sheet = sheet_of(
            &[("Иванов", vec![graded(8, 10, Some((2, WorkScore)))])],
            &columns,
        );
        let label = "6 (−2 из балла, из 10)";
        assert_eq!(cell_label(&sheet.groups[0].students[0].cells[0]), label);

        let widths = column_widths(&sheet.groups[0]);

        assert!(widths[FIRST_ASSIGNMENT_COLUMN as usize] >= chars(label));
    }

    #[test]
    fn a_long_student_name_does_not_widen_the_frozen_column_past_its_limit() {
        let name = "Я".repeat(100);
        let columns = [column("ДЗ", Some(10), None)];
        let sheet = sheet_of(&[(name.as_str(), vec![graded(5, 10, None)])], &columns);

        assert_eq!(column_widths(&sheet.groups[0])[0], STUDENT_MAX_WIDTH);
    }

    #[test]
    fn a_group_without_assignments_still_builds_a_valid_sheet() {
        let mut sheet = sample();
        sheet.groups[0].columns.clear();
        for student in &mut sheet.groups[0].students {
            student.cells.clear();
        }

        let mut book = workbook(&built(&sheet));

        let range = book.worksheet_range("Б9123").unwrap();
        assert_eq!(value(&range, HEADER_ROW, 1), text("Сдано\nиз 0"));
    }

    fn names(codes: &[&str]) -> Vec<String> {
        sheet_names(codes.iter().copied())
    }

    #[test]
    fn forbidden_characters_leave_the_sheet_name() {
        let [name] = names(&["Б91/23\\:*?[]"]).try_into().unwrap();

        assert!(!name.contains(SHEET_NAME_FORBIDDEN));
        assert!(name.starts_with("Б91"));
    }

    #[test]
    fn apostrophes_do_not_stay_at_the_edges() {
        assert_eq!(names(&["'Б91'"]), ["Б91"]);
        assert_eq!(names(&["''"]), [texts::SHEET_FALLBACK_NAME]);
    }

    #[test]
    fn a_name_is_cut_to_the_limit_in_utf16_units() {
        let [name] = names(&["😀".repeat(40).as_str()]).try_into().unwrap();

        assert!(name.encode_utf16().count() <= SHEET_NAME_MAX_UTF16);
        assert!(name.chars().all(|character| character == '😀'));
    }

    #[test]
    fn a_cut_that_ends_on_an_apostrophe_drops_it() {
        let code = format!("{}'{}", "Б".repeat(30), "В".repeat(5));

        let [name] = names(&[code.as_str()]).try_into().unwrap();

        assert_eq!(name, "Б".repeat(30));
    }

    #[test]
    fn the_reserved_name_is_not_used() {
        for code in ["History", "history", "HISTORY"] {
            let [name] = names(&[code]).try_into().unwrap();
            assert_ne!(name.to_lowercase(), "history");
        }
    }

    #[test]
    fn codes_equal_after_the_cut_or_by_case_get_different_names() {
        let long = "Г".repeat(40);
        let other = format!("{}x", "Г".repeat(39));

        let cut = names(&[long.as_str(), other.as_str()]);
        let cased = names(&["abc", "ABC"]);

        assert_ne!(cut[0].to_lowercase(), cut[1].to_lowercase());
        assert!(cut[1].encode_utf16().count() <= SHEET_NAME_MAX_UTF16);
        assert_ne!(cased[0].to_lowercase(), cased[1].to_lowercase());
    }

    #[test]
    fn every_generated_name_is_accepted_by_the_workbook() {
        let codes = [
            "Б91/23",
            "'quoted'",
            "History",
            "abc",
            "ABC",
            &"Д".repeat(64),
            &"😀".repeat(40),
        ];
        let mut workbook = Workbook::new();

        for name in sheet_names(codes.iter().copied()) {
            workbook.add_worksheet().set_name(name).unwrap();
        }

        workbook.save_to_buffer().unwrap();
    }

    #[test]
    fn the_file_name_holds_the_subject_and_the_date() {
        let file = built(&sample());

        assert_eq!(file.filename, "Ведомость Алгоритмы 23.11.2026.xlsx");
    }

    #[test]
    fn forbidden_characters_leave_the_file_name() {
        let name = file_name("Сети/Базы \"данных\": <1>", "23.11.2026");

        assert!(!name.contains(FILE_NAME_FORBIDDEN));
        assert!(name.starts_with("Ведомость Сети"));
        assert!(name.ends_with("23.11.2026.xlsx"));
    }

    #[test]
    fn a_subject_of_only_forbidden_characters_leaves_a_clean_name() {
        assert_eq!(file_name("///", "23.11.2026"), "Ведомость 23.11.2026.xlsx");
    }

    #[test]
    fn a_long_subject_fits_the_file_name_limit() {
        let name = file_name(&"Я".repeat(200), "23.11.2026");

        assert!(name.chars().count() <= FILE_NAME_MAX_LEN);
        assert!(name.len() <= FILE_NAME_MAX_LEN);
        assert!(name.ends_with("23.11.2026.xlsx"));
    }
}
