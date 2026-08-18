use super::views::BUTTON_QUEUE;

pub const GUEST_NEEDS_INVITE: &str = "Регистрация только по ссылке-приглашению группы. Попросите её у преподавателя и откройте \
     присланную ссылку.";
pub const USE_THE_MENU: &str = "Используйте меню под полем ввода или команду /menu.";
pub const SUBMISSION_PROMPT: &str = "Пришлите ссылку или текст сдачи одним сообщением:";
pub const STUDENT_QUEUE_EMPTY: &str = "⏳ Сейчас ни одна ваша работа не ждёт проверки.";
pub const REVIEW_QUEUE_EMPTY: &str = "🎉 Очередь проверки пуста.";
pub const FILE_DOWNLOAD_FAILED: &str = "Не удалось скачать файл. Попробуйте ещё раз.";
pub const STALE_BUTTON: &str =
    "Эта кнопка из старого сообщения, действие не выполнено. Откройте /menu.";

pub const FILE_DELETE_CONFIRM: &str = "Удалить вложение из уже выданного задания?";
pub const LATE_POLICY_QUESTION: &str = "Штраф за опоздание?";
pub const SUBMISSIONS_CLOSE_LABEL: &str = "🚪 Приём закрыт после:";
pub const LATE_PENALTY_LABEL: &str = "⏳ Штраф за опоздание сейчас:";

const CANCEL_HINT: &str = "/cancel — отменить";

pub fn input_prompt(prompt: &str) -> String {
    format!("{prompt}\n\n{CANCEL_HINT}")
}

pub fn submission_queued() -> String {
    format!("✅ Работа отправлена на проверку. Место в очереди видно в «{BUTTON_QUEUE}».")
}
