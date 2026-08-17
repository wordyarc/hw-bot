create table student_group_invites (
    student_group_id uuid primary key
        references student_groups (id) on delete cascade,
    generation bigint not null check (generation > 0 and generation <= 4294967295),
    updated_at timestamptz not null default now()
);

insert into student_group_invites (student_group_id, generation)
select id, 1
from student_groups;
