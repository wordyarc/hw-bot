\pset footer off

\echo 'Submissions waiting for review longer than 7 days'
select count(*) as submissions, min(submitted_at) as oldest
from submissions
where status = 'pending_review'
  and submitted_at < now() - interval '7 days';

\echo 'Notifications undelivered for more than 15 minutes'
select count(*) as notifications, min(created_at) as oldest
from review_notification_outbox
where delivered_at is null
  and failed_at is null
  and created_at < now() - interval '15 minutes';

\echo 'Notifications that failed permanently'
select count(*) as notifications, min(failed_at) as oldest
from review_notification_outbox
where failed_at is not null;
