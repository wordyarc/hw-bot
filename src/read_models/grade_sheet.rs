use crate::domain::{AssignmentId, DeductedFrom, Grade, Grading, StudentGroupId, Subject, UserId};

#[derive(Clone, Debug)]
pub struct GradeSheet {
    pub subject: Subject,
    pub groups: Vec<GradeSheetGroup>,
}

#[derive(Clone, Debug)]
pub struct GradeSheetGroup {
    pub id: StudentGroupId,
    pub code: String,
    pub columns: Vec<GradeSheetColumn>,
    pub students: Vec<GradeSheetStudent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GradeSheetColumn {
    pub assignment_id: AssignmentId,
    pub title: String,
    pub grading: Grading,
    pub deducted_from: Option<DeductedFrom>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GradeSheetStudent {
    pub student_id: UserId,
    pub display_name: String,
    pub cells: Vec<GradeSheetCell>,
    pub accepted: u32,
    pub pending: u32,
    pub total: i64,
    pub max_total: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GradeSheetCell {
    Score(ScoreCell),
    Passed { makeup: bool },
    Pending,
    Rejected,
    Empty,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScoreCell {
    pub contribution: i32,
    pub late: Option<LateMark>,
    pub makeup: bool,
    pub frozen_scale: Option<u16>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LateMark {
    Removed {
        points: u16,
        mode: Option<DeductedFrom>,
    },
    NothingRemoved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AcceptedWork {
    pub grade: Option<Grade>,
    pub makeup: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SubmissionStates {
    pub accepted: Option<AcceptedWork>,
    pub pending: bool,
    pub rejected: bool,
}

#[derive(Clone, Debug)]
pub struct GradeSheetRow {
    pub group_id: StudentGroupId,
    pub group_code: String,
    pub student_id: UserId,
    pub display_name: String,
    pub assignment: Option<(GradeSheetColumn, SubmissionStates)>,
}

impl GradeSheet {
    pub fn assemble(subject: Subject, rows: Vec<GradeSheetRow>) -> Option<Self> {
        let mut groups: Vec<GradeSheetGroup> = Vec::new();
        for row in rows {
            if groups.last().is_none_or(|group| group.id != row.group_id) {
                groups.push(GradeSheetGroup {
                    id: row.group_id,
                    code: row.group_code,
                    columns: Vec::new(),
                    students: Vec::new(),
                });
            }
            let group = groups.last_mut()?;
            if group
                .students
                .last()
                .is_none_or(|student| student.student_id != row.student_id)
            {
                group
                    .students
                    .push(GradeSheetStudent::new(row.student_id, row.display_name));
            }
            let first_student = group.students.len() == 1;
            let student = group.students.last_mut()?;
            if let Some((column, states)) = row.assignment {
                student.add(&column, states);
                if first_student {
                    group.columns.push(column);
                }
            }
        }

        (!groups.is_empty()).then_some(Self { subject, groups })
    }
}

impl GradeSheetStudent {
    fn new(student_id: UserId, display_name: String) -> Self {
        Self {
            student_id,
            display_name,
            cells: Vec::new(),
            accepted: 0,
            pending: 0,
            total: 0,
            max_total: 0,
        }
    }

    fn add(&mut self, column: &GradeSheetColumn, states: SubmissionStates) {
        let cell = GradeSheetCell::of(column, states);
        if let GradeSheetCell::Score(score) = cell {
            self.total += i64::from(score.contribution);
        }
        let max_score = match states.accepted {
            Some(work) => work.grade.map_or(0, Grade::max_score),
            None => column.grading.max_score().unwrap_or(0),
        };
        self.max_total += u32::from(max_score);
        self.accepted += u32::from(states.accepted.is_some());
        self.pending += u32::from(states.pending);
        self.cells.push(cell);
    }
}

impl GradeSheetCell {
    fn of(column: &GradeSheetColumn, states: SubmissionStates) -> Self {
        match states.accepted {
            Some(AcceptedWork {
                grade: Some(grade),
                makeup,
            }) => Self::Score(ScoreCell::new(column, grade, makeup)),
            Some(AcceptedWork {
                grade: None,
                makeup,
            }) => Self::Passed { makeup },
            None if states.pending => Self::Pending,
            None if states.rejected => Self::Rejected,
            None => Self::Empty,
        }
    }
}

impl ScoreCell {
    fn new(column: &GradeSheetColumn, grade: Grade, makeup: bool) -> Self {
        let removed = grade.raw_score() - grade.work_score() + grade.course_total_penalty();
        let late = grade.deducted_from().map(|deducted_from| match removed {
            0 => LateMark::NothingRemoved,
            points => LateMark::Removed {
                points,
                mode: (column.deducted_from != Some(deducted_from)).then_some(deducted_from),
            },
        });

        Self {
            contribution: i32::from(grade.work_score()) - i32::from(grade.course_total_penalty()),
            late,
            makeup,
            frozen_scale: (column.grading.max_score() != Some(grade.max_score()))
                .then_some(grade.max_score()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::DeductedFrom::{CourseTotal, WorkScore};
    use crate::domain::LatePenalty;
    use crate::test_support::subject;

    fn column(grading: Grading, deducted_from: Option<DeductedFrom>) -> GradeSheetColumn {
        GradeSheetColumn {
            assignment_id: AssignmentId::new(),
            title: "ДЗ".to_owned(),
            grading,
            deducted_from,
        }
    }

    fn out_of(max_score: u16) -> Grading {
        Grading::scored(max_score).unwrap()
    }

    fn on_time(raw: u16, max: u16) -> Grade {
        Grade::new(raw, max, None)
    }

    fn late(raw: u16, max: u16, points: u16, deducted_from: DeductedFrom) -> Grade {
        Grade::new(raw, max, LatePenalty::restore(1, points, deducted_from))
    }

    fn accepted(grade: Grade) -> SubmissionStates {
        SubmissionStates {
            accepted: Some(AcceptedWork {
                grade: Some(grade),
                makeup: false,
            }),
            ..SubmissionStates::default()
        }
    }

    fn passed(makeup: bool) -> SubmissionStates {
        SubmissionStates {
            accepted: Some(AcceptedWork {
                grade: None,
                makeup,
            }),
            ..SubmissionStates::default()
        }
    }

    fn score(column: &GradeSheetColumn, states: SubmissionStates) -> ScoreCell {
        match GradeSheetCell::of(column, states) {
            GradeSheetCell::Score(cell) => cell,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_accepted_work_outranks_a_pending_and_a_rejected_one() {
        let column = column(out_of(10), None);
        let states = SubmissionStates {
            pending: true,
            rejected: true,
            ..accepted(on_time(8, 10))
        };

        assert!(matches!(
            GradeSheetCell::of(&column, states),
            GradeSheetCell::Score(ScoreCell {
                contribution: 8,
                ..
            })
        ));
    }

    #[test]
    fn a_pending_work_outranks_a_rejected_one() {
        let states = SubmissionStates {
            pending: true,
            rejected: true,
            accepted: None,
        };

        assert_eq!(
            GradeSheetCell::of(&column(out_of(10), None), states),
            GradeSheetCell::Pending
        );
    }

    #[test]
    fn a_rejected_work_stays_visible_when_nothing_newer_counts() {
        let states = SubmissionStates {
            rejected: true,
            ..SubmissionStates::default()
        };

        assert_eq!(
            GradeSheetCell::of(&column(out_of(10), None), states),
            GradeSheetCell::Rejected
        );
    }

    #[test]
    fn without_a_counted_submission_the_cell_is_empty() {
        assert_eq!(
            GradeSheetCell::of(&column(out_of(10), None), SubmissionStates::default()),
            GradeSheetCell::Empty
        );
    }

    #[test]
    fn a_pass_keeps_its_makeup_mark() {
        let column = column(Grading::PassFail, None);

        assert_eq!(
            GradeSheetCell::of(&column, passed(false)),
            GradeSheetCell::Passed { makeup: false }
        );
        assert_eq!(
            GradeSheetCell::of(&column, passed(true)),
            GradeSheetCell::Passed { makeup: true }
        );
    }

    #[test]
    fn a_grade_on_time_contributes_its_score_without_marks() {
        let column = column(out_of(10), Some(CourseTotal));

        assert_eq!(
            score(&column, accepted(on_time(10, 10))),
            ScoreCell {
                contribution: 10,
                late: None,
                makeup: false,
                frozen_scale: None,
            }
        );
    }

    #[test]
    fn both_penalty_modes_contribute_the_score_minus_the_penalty() {
        for mode in [WorkScore, CourseTotal] {
            let column = column(out_of(10), Some(mode));

            let cell = score(&column, accepted(late(8, 10, 2, mode)));

            assert_eq!(cell.contribution, 6, "{mode:?}");
            assert_eq!(
                cell.late,
                Some(LateMark::Removed {
                    points: 2,
                    mode: None
                }),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn a_penalty_above_the_score_removes_only_the_score_from_the_work() {
        let column = column(out_of(10), Some(WorkScore));

        let cell = score(&column, accepted(late(1, 10, 2, WorkScore)));

        assert_eq!(cell.contribution, 0);
        assert_eq!(
            cell.late,
            Some(LateMark::Removed {
                points: 1,
                mode: None
            })
        );
    }

    #[test]
    fn a_penalty_above_the_score_goes_below_zero_in_the_course_total() {
        let column = column(out_of(10), Some(CourseTotal));

        let cell = score(&column, accepted(late(1, 10, 2, CourseTotal)));

        assert_eq!(cell.contribution, -1);
        assert_eq!(
            cell.late,
            Some(LateMark::Removed {
                points: 2,
                mode: None
            })
        );
    }

    #[test]
    fn a_zero_with_a_penalty_from_the_work_has_nothing_to_remove() {
        let column = column(out_of(10), Some(CourseTotal));

        let cell = score(&column, accepted(late(0, 10, 2, WorkScore)));

        assert_eq!(cell.contribution, 0);
        assert_eq!(cell.late, Some(LateMark::NothingRemoved));
    }

    #[test]
    fn a_mode_changed_after_the_review_is_named_in_the_cell() {
        let column = column(out_of(10), Some(CourseTotal));

        let cell = score(&column, accepted(late(8, 10, 2, WorkScore)));

        assert_eq!(
            cell.late,
            Some(LateMark::Removed {
                points: 2,
                mode: Some(WorkScore)
            })
        );
    }

    #[test]
    fn a_policy_removed_after_the_review_names_the_mode_of_the_grade() {
        let column = column(out_of(10), None);

        let cell = score(&column, accepted(late(8, 10, 2, CourseTotal)));

        assert_eq!(
            cell.late,
            Some(LateMark::Removed {
                points: 2,
                mode: Some(CourseTotal)
            })
        );
    }

    #[test]
    fn a_makeup_grade_is_marked() {
        let column = column(out_of(10), None);
        let states = SubmissionStates {
            accepted: Some(AcceptedWork {
                grade: Some(on_time(6, 10)),
                makeup: true,
            }),
            ..SubmissionStates::default()
        };

        assert!(score(&column, states).makeup);
    }

    #[test]
    fn a_grade_on_a_previous_scale_names_that_scale() {
        let column = column(out_of(20), None);

        let cell = score(&column, accepted(on_time(8, 10)));

        assert_eq!(cell.contribution, 8);
        assert_eq!(cell.frozen_scale, Some(10));
    }

    #[test]
    fn a_grade_in_an_assignment_that_became_pass_fail_names_its_scale() {
        let column = column(Grading::PassFail, None);

        let cell = score(&column, accepted(on_time(8, 10)));

        assert_eq!(cell.frozen_scale, Some(10));
    }

    fn row(
        group: (StudentGroupId, &str),
        student: (UserId, &str),
        assignment: Option<(&GradeSheetColumn, SubmissionStates)>,
    ) -> GradeSheetRow {
        GradeSheetRow {
            group_id: group.0,
            group_code: group.1.to_owned(),
            student_id: student.0,
            display_name: student.1.to_owned(),
            assignment: assignment.map(|(column, states)| (column.clone(), states)),
        }
    }

    fn one_student(cells: Vec<(GradeSheetColumn, SubmissionStates)>) -> GradeSheetStudent {
        let group = (StudentGroupId::new(), "Б9123");
        let student = (UserId::new(), "Иванов");
        let rows = cells
            .iter()
            .map(|(column, states)| row(group, student, Some((column, *states))))
            .collect();

        let mut sheet = GradeSheet::assemble(subject("Алгоритмы"), rows).unwrap();
        sheet.groups.remove(0).students.remove(0)
    }

    #[test]
    fn no_rows_make_no_sheet() {
        assert!(GradeSheet::assemble(subject("Алгоритмы"), Vec::new()).is_none());
    }

    #[test]
    fn rows_become_groups_students_and_shared_columns_in_their_order() {
        let first = (StudentGroupId::new(), "ИВТ-21");
        let second = (StudentGroupId::new(), "ИВТ-22");
        let (anna, boris, vera) = (
            (UserId::new(), "Анна"),
            (UserId::new(), "Борис"),
            (UserId::new(), "Вера"),
        );
        let (one, two) = (column(out_of(10), None), column(Grading::PassFail, None));
        let empty = SubmissionStates::default();
        let rows = vec![
            row(first, anna, Some((&one, empty))),
            row(first, anna, Some((&two, empty))),
            row(first, boris, Some((&one, empty))),
            row(first, boris, Some((&two, empty))),
            row(second, vera, None),
        ];

        let sheet = GradeSheet::assemble(subject("Алгоритмы"), rows).unwrap();

        assert_eq!(sheet.groups.len(), 2);
        assert_eq!(sheet.groups[0].code, "ИВТ-21");
        assert_eq!(sheet.groups[0].columns, [one, two]);
        let names: Vec<&str> = sheet.groups[0]
            .students
            .iter()
            .map(|student| student.display_name.as_str())
            .collect();
        assert_eq!(names, ["Анна", "Борис"]);
        assert_eq!(sheet.groups[0].students[1].cells.len(), 2);
        assert!(sheet.groups[1].columns.is_empty());
        assert!(sheet.groups[1].students[0].cells.is_empty());
    }

    #[test]
    fn a_pass_counts_as_accepted_but_stays_out_of_the_sum_and_the_maximum() {
        let student = one_student(vec![
            (column(out_of(10), None), accepted(on_time(7, 10))),
            (column(Grading::PassFail, None), passed(false)),
        ]);

        assert_eq!(
            (student.accepted, student.total, student.max_total),
            (2, 7, 10)
        );
    }

    #[test]
    fn a_pass_in_an_assignment_that_became_scored_adds_nothing_to_the_maximum() {
        let student = one_student(vec![(column(out_of(10), None), passed(false))]);

        assert_eq!((student.total, student.max_total), (0, 0));
    }

    #[test]
    fn the_sum_goes_below_zero_with_a_penalty_from_the_total() {
        let student = one_student(vec![(
            column(out_of(10), Some(CourseTotal)),
            accepted(late(0, 10, 4, CourseTotal)),
        )]);

        assert_eq!((student.total, student.max_total), (-4, 10));
    }

    #[test]
    fn the_maximum_uses_the_frozen_scale_of_a_grade_and_the_current_one_otherwise() {
        let student = one_student(vec![
            (column(out_of(20), None), accepted(on_time(8, 10))),
            (column(out_of(5), None), SubmissionStates::default()),
            (column(Grading::PassFail, None), SubmissionStates::default()),
        ]);

        assert_eq!((student.total, student.max_total), (8, 15));
    }

    #[test]
    fn a_pending_makeup_over_an_accepted_work_is_counted_in_both_columns() {
        let states = SubmissionStates {
            pending: true,
            ..accepted(on_time(8, 10))
        };

        let student = one_student(vec![(column(out_of(10), None), states)]);

        assert_eq!(
            (student.accepted, student.pending, student.total),
            (1, 1, 8)
        );
    }

    #[test]
    fn a_pending_work_is_not_accepted() {
        let states = SubmissionStates {
            pending: true,
            ..SubmissionStates::default()
        };

        let student = one_student(vec![(column(out_of(10), None), states)]);

        assert_eq!(
            (
                student.accepted,
                student.pending,
                student.total,
                student.max_total
            ),
            (0, 1, 0, 10)
        );
    }
}
