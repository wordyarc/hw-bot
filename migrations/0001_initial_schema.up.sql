-- Student groups and subjects.

create table student_groups (
    id uuid primary key default gen_random_uuid(),
    code text not null unique check (btrim(code) <> '' and char_length(code) <= 64),
    education_level text not null check (
        education_level in ('bachelor', 'master', 'specialist', 'postgraduate')
    ),
    study_year smallint not null check (study_year > 0),
    created_at timestamptz not null default now(),
    check (
        (education_level = 'bachelor' and study_year <= 4)
        or (education_level = 'master' and study_year <= 2)
        or (education_level = 'specialist' and study_year <= 6)
        or (education_level = 'postgraduate' and study_year <= 4)
    )
);

create table subjects (
    id uuid primary key default gen_random_uuid(),
    name text not null unique check (btrim(name) <> '' and char_length(name) <= 200),
    created_at timestamptz not null default now()
);

-- Users.

create table users (
    id uuid primary key default uuidv7(),
    telegram_user_id bigint not null unique check (telegram_user_id > 0),
    role text not null check (role in ('student', 'teacher', 'admin')),
    display_name text not null check (
        btrim(display_name) <> '' and char_length(display_name) <= 100
    ),
    student_group_id uuid references student_groups (id) on delete restrict,
    registered_at timestamptz not null default now(),
    check ((role = 'student') = (student_group_id is not null))
);

create index users_student_group_id_idx on users (student_group_id)
where student_group_id is not null;

-- Teaching courses and course teachers.

create table teaching_courses (
    id uuid primary key default uuidv7(),
    subject_id uuid not null references subjects (id) on delete restrict,
    student_group_id uuid not null references student_groups (id) on delete restrict,
    created_at timestamptz not null default now(),
    unique (subject_id, student_group_id),
    unique (id, subject_id, student_group_id)
);

create index teaching_courses_student_group_id_idx
    on teaching_courses (student_group_id);

create table teaching_course_teachers (
    teaching_course_id uuid not null references teaching_courses (id) on delete cascade,
    teacher_id uuid not null references users (id) on delete restrict,
    added_at timestamptz not null default now(),
    primary key (teaching_course_id, teacher_id)
);

create index teaching_course_teachers_teacher_id_idx
    on teaching_course_teachers (teacher_id);

-- Assignments.

create table assignments (
    id uuid primary key default uuidv7(),
    teaching_course_id uuid not null,
    subject_id uuid not null,
    student_group_id uuid not null,
    title text not null check (btrim(title) <> '' and char_length(title) <= 200),
    body text not null check (btrim(body) <> '' and char_length(body) <= 4096),
    max_attempts smallint not null check (max_attempts between 1 and 32767),
    status text not null check (status in ('draft', 'published', 'archived')),
    created_by uuid not null references users (id) on delete restrict,
    created_at timestamptz not null default now(),
    deadline_at timestamptz not null,
    published_at timestamptz,
    revision bigint not null default 0 check (revision >= 0),
    check (deadline_at > created_at),
    check (
        (status = 'draft' and published_at is null)
        or (status = 'published' and published_at is not null)
        or status = 'archived'
    ),
    foreign key (teaching_course_id, subject_id, student_group_id)
        references teaching_courses (id, subject_id, student_group_id)
        on delete restrict
);

create index assignments_teaching_course_id_idx
    on assignments (teaching_course_id);

create index assignments_student_group_status_idx
    on assignments (student_group_id, status);

-- Submissions and review queue.

create table submissions (
    id uuid primary key default uuidv7(),
    assignment_id uuid not null references assignments (id) on delete restrict,
    student_id uuid not null references users (id) on delete restrict,
    attempt_number smallint not null check (attempt_number between 1 and 32767),
    content text not null check (btrim(content) <> '' and char_length(content) <= 4096),
    status text not null check (status in ('pending_review', 'accepted', 'rejected')),
    submitted_at timestamptz not null default now(),
    reviewed_by uuid references users (id) on delete restrict,
    reviewed_at timestamptz,
    review_comment text,
    unique (assignment_id, student_id, attempt_number),
    check (
        (
            status = 'pending_review'
            and reviewed_by is null
            and reviewed_at is null
            and review_comment is null
        )
        or (
            status in ('accepted', 'rejected')
            and reviewed_by is not null
            and reviewed_at is not null
            and review_comment is not null
            and btrim(review_comment) <> ''
        )
    ),
    check (review_comment is null or char_length(review_comment) <= 4096)
);

create unique index submissions_one_pending_per_student_assignment_idx
    on submissions (assignment_id, student_id)
    where status = 'pending_review';

create unique index submissions_one_accepted_per_student_assignment_idx
    on submissions (assignment_id, student_id)
    where status = 'accepted';

create index submissions_pending_review_queue_idx
    on submissions (submitted_at, id)
    where status = 'pending_review';

create index submissions_student_assignment_idx
    on submissions (student_id, assignment_id);

-- Assignment and submission attachments. Installation-specific size and count limits live in app.

create table stored_files (
    id uuid primary key default uuidv7(),
    assignment_id uuid references assignments (id) on delete restrict,
    submission_id uuid references submissions (id) on delete restrict,
    position bigint not null check (position > 0),
    presentation text not null check (presentation in ('document', 'image')),
    filename text not null check (btrim(filename) <> ''),
    mime_type text check (mime_type is null or btrim(mime_type) <> ''),
    size_bytes bigint not null check (size_bytes >= 0),
    sha256 bytea not null check (octet_length(sha256) = 32),
    content bytea not null,
    telegram_file_id text check (telegram_file_id is null or btrim(telegram_file_id) <> ''),
    created_at timestamptz not null default now(),
    check (num_nonnulls(assignment_id, submission_id) = 1),
    check (octet_length(content)::bigint = size_bytes)
);

create unique index stored_files_assignment_position_idx
    on stored_files (assignment_id, position)
    where assignment_id is not null;

create unique index stored_files_submission_position_idx
    on stored_files (submission_id, position)
    where submission_id is not null;

-- Per-student assignment progress.

create table student_assignment_progress (
    assignment_id uuid not null references assignments (id) on delete restrict,
    student_id uuid not null references users (id) on delete restrict,
    extra_attempts smallint not null default 0 check (extra_attempts between 0 and 32767),
    primary key (assignment_id, student_id)
);
