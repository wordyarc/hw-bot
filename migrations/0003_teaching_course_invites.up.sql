create table teaching_course_invites (
    teaching_course_id uuid primary key
        references teaching_courses (id) on delete cascade,
    generation bigint not null check (generation > 0 and generation <= 4294967295),
    updated_at timestamptz not null default now()
);

insert into teaching_course_invites (teaching_course_id, generation)
select id, 1
from teaching_courses;
