create table teaching_course_students (
    teaching_course_id uuid not null references teaching_courses (id) on delete cascade,
    student_id uuid not null references users (id) on delete restrict,
    enrolled_at timestamptz not null default now(),
    primary key (teaching_course_id, student_id)
);

insert into teaching_course_students (teaching_course_id, student_id)
select course_groups.teaching_course_id, users.id
from users
join teaching_course_groups course_groups
  on course_groups.student_group_id = users.student_group_id
join subjects
  on subjects.id = course_groups.subject_id
where users.role = 'student'
  and not subjects.sandbox;

create view course_participants as
select
    course_groups.teaching_course_id,
    users.id as student_id,
    users.student_group_id
from teaching_course_groups course_groups
join subjects
  on subjects.id = course_groups.subject_id
join users
  on users.student_group_id = course_groups.student_group_id
 and users.role = 'student'
where subjects.sandbox
   or exists (
       select 1
       from teaching_course_students enrolled
       where enrolled.teaching_course_id = course_groups.teaching_course_id
         and enrolled.student_id = users.id
   );
