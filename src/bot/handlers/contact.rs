use reqwest::Url;
use teloxide::payloads::SendMessageSetters;
use teloxide::types::{ChatFullInfo, ChatFullInfoKind, ChatId, MessageId, ReplyParameters};
use teloxide::{ApiError, RequestError};

use crate::app::StudentContact;
use crate::bot::HandlerResult;
use crate::bot::callbacks::InformationalAction;
use crate::bot::context::Screen;
use crate::bot::errors::log_request_error;
use crate::bot::{texts, views};
use crate::domain::{TeachingCourseId, TelegramUserId, UserId};

pub(super) async fn show(
    screen: Screen<'_>,
    source: MessageId,
    course_id: TeachingCourseId,
    student_id: UserId,
) -> HandlerResult {
    let student = screen
        .services()
        .courses
        .student_contact(screen.actor(), course_id, student_id)
        .await?;
    let contact = match screen
        .bot()
        .get_chat(ChatId(student.telegram_user_id.as_i64()))
        .await
    {
        Ok(chat) => {
            let contact = TelegramContact::from_chat(chat, student.telegram_user_id);
            if contact == TelegramContact::LookupFailed {
                tracing::warn!(
                    operation = "student_contact",
                    kind = "telegram_unexpected_chat",
                    "Telegram contact lookup failed"
                );
            }
            contact
        }
        Err(error) => {
            log_request_error("student_contact", &error);
            TelegramContact::from_error(error)?
        }
    };
    let target = InformationalAction::StudentContact {
        course_id,
        student_id: student.student_id,
    };
    match send_card(screen, source, &student, target, &contact).await {
        Err(error)
            if matches!(contact, TelegramContact::UserId(_))
                && is_user_privacy_restriction(&error) =>
        {
            log_request_error("student_contact_send", &error);
            send_card(
                screen,
                source,
                &student,
                target,
                &TelegramContact::Restricted,
            )
            .await?;
        }
        result => result?,
    }
    Ok(())
}

async fn send_card(
    screen: Screen<'_>,
    source: MessageId,
    student: &StudentContact,
    target: InformationalAction,
    contact: &TelegramContact,
) -> Result<(), RequestError> {
    let detail = match contact {
        TelegramContact::Username(username) => texts::contact_username(username),
        TelegramContact::UserId(_) => texts::CONTACT_NO_USERNAME.to_owned(),
        TelegramContact::Restricted => texts::CONTACT_RESTRICTED.to_owned(),
        TelegramContact::Unavailable => texts::CONTACT_UNAVAILABLE.to_owned(),
        TelegramContact::LookupFailed => texts::CONTACT_LOOKUP_FAILED.to_owned(),
    };
    let mut text = format!(
        "{}\n{detail}",
        texts::student_contact_header(&student.display_name, &student.group_code),
    );
    let url = contact.url();
    if url.is_some() {
        text.push_str("\n\n");
        text.push_str(texts::CONTACT_PERSONAL_ACCOUNT);
    }
    let refresh = if matches!(contact, TelegramContact::LookupFailed) {
        texts::BUTTON_RETRY_CONTACT
    } else {
        texts::BUTTON_REFRESH_CONTACT
    };
    screen
        .bot()
        .send_message(screen.chat_id(), text)
        .reply_parameters(ReplyParameters::new(source).allow_sending_without_reply())
        .reply_markup(views::contact_keyboard(target, url, refresh))
        .await?;
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum TelegramContact {
    Username(String),
    UserId(TelegramUserId),
    Restricted,
    Unavailable,
    LookupFailed,
}

impl TelegramContact {
    fn from_chat(chat: ChatFullInfo, expected: TelegramUserId) -> Self {
        if chat.id != ChatId(expected.as_i64()) {
            return Self::LookupFailed;
        }
        let ChatFullInfoKind::Private(private) = chat.kind else {
            return Self::LookupFailed;
        };
        match private.username {
            Some(username) => Self::Username(username),
            None if private.has_private_forwards => Self::Restricted,
            None => Self::UserId(expected),
        }
    }

    fn from_error(error: RequestError) -> Result<Self, RequestError> {
        match error {
            RequestError::Api(ApiError::InvalidToken) => Err(error),
            RequestError::Api(
                ApiError::ChatNotFound
                | ApiError::UserNotFound
                | ApiError::UserDeactivated
                | ApiError::BotBlocked,
            ) => Ok(Self::Unavailable),
            _ => Ok(Self::LookupFailed),
        }
    }

    fn url(&self) -> Option<Url> {
        match self {
            Self::Username(username) => {
                let mut url = Url::parse("https://t.me/").expect("valid Telegram URL");
                url.path_segments_mut()
                    .expect("Telegram URL supports path segments")
                    .push(username);
                Some(url)
            }
            Self::UserId(id) => {
                let mut url = Url::parse("tg://user").expect("valid Telegram user URL");
                url.query_pairs_mut()
                    .append_pair("id", &id.as_i64().to_string());
                Some(url)
            }
            Self::Restricted | Self::Unavailable | Self::LookupFailed => None,
        }
    }
}

fn is_user_privacy_restriction(error: &RequestError) -> bool {
    matches!(error, RequestError::Api(ApiError::Unknown(description))
        if description == "Bad Request: BUTTON_USER_PRIVACY_RESTRICTED")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::telegram::private_chat_info;

    fn id() -> TelegramUserId {
        TelegramUserId::new(12345).unwrap()
    }

    fn contact(username: Option<&str>, restricted: bool) -> TelegramContact {
        TelegramContact::from_chat(
            serde_json::from_value(private_chat_info(id().as_i64(), username, restricted)).unwrap(),
            id(),
        )
    }

    #[test]
    fn public_username_is_preferred_regardless_of_private_forwards() {
        for restricted in [false, true] {
            let contact = contact(Some("student_name"), restricted);
            assert_eq!(contact, TelegramContact::Username("student_name".into()));
            assert_eq!(contact.url().unwrap().as_str(), "https://t.me/student_name");
        }
    }

    #[test]
    fn no_username_uses_the_stable_id_when_allowed() {
        let contact = contact(None, false);
        assert_eq!(contact, TelegramContact::UserId(id()));
        assert_eq!(contact.url().unwrap().as_str(), "tg://user?id=12345");
    }

    #[test]
    fn private_forwards_without_username_prevents_a_link() {
        let contact = contact(None, true);
        assert_eq!(contact, TelegramContact::Restricted);
        assert!(contact.url().is_none());
    }

    #[test]
    fn absent_private_forwards_uses_telegram_default() {
        let mut info = private_chat_info(id().as_i64(), None, false);
        info.as_object_mut().unwrap().remove("has_private_forwards");
        assert_eq!(
            TelegramContact::from_chat(serde_json::from_value(info).unwrap(), id()),
            TelegramContact::UserId(id())
        );
    }

    #[test]
    fn unexpected_chat_id_is_not_used_as_a_contact() {
        let chat = private_chat_info(67890, Some("someone_else"), false);
        let contact = TelegramContact::from_chat(serde_json::from_value(chat).unwrap(), id());
        assert_eq!(contact, TelegramContact::LookupFailed);
        assert!(contact.url().is_none());
    }

    #[test]
    fn public_chat_is_not_used_as_a_contact_even_with_matching_id() {
        let mut chat = private_chat_info(id().as_i64(), Some("channel"), false);
        chat["type"] = "channel".into();
        assert_eq!(
            TelegramContact::from_chat(serde_json::from_value(chat).unwrap(), id()),
            TelegramContact::LookupFailed
        );
    }

    #[test]
    fn known_account_errors_mean_unavailable_not_privacy() {
        for error in [
            ApiError::ChatNotFound,
            ApiError::UserNotFound,
            ApiError::UserDeactivated,
            ApiError::BotBlocked,
        ] {
            let contact = TelegramContact::from_error(RequestError::Api(error)).unwrap();
            assert_eq!(contact, TelegramContact::Unavailable);
            assert!(contact.url().is_none());
        }
    }

    #[test]
    fn unexpected_and_temporary_lookup_errors_allow_retry() {
        for error in [
            RequestError::Api(ApiError::Unknown("unexpected".into())),
            RequestError::Io(std::sync::Arc::new(std::io::ErrorKind::TimedOut.into())),
            RequestError::RetryAfter(teloxide::types::Seconds::from_seconds(3)),
        ] {
            let contact = TelegramContact::from_error(error).unwrap();
            assert_eq!(contact, TelegramContact::LookupFailed);
            assert!(contact.url().is_none());
        }
    }

    #[test]
    fn invalid_credentials_remain_a_typed_fatal_error() {
        assert!(matches!(
            TelegramContact::from_error(RequestError::Api(ApiError::InvalidToken)),
            Err(RequestError::Api(ApiError::InvalidToken))
        ));
    }

    #[test]
    fn only_the_exact_user_privacy_error_allows_fallback() {
        assert!(is_user_privacy_restriction(&RequestError::Api(
            ApiError::Unknown("Bad Request: BUTTON_USER_PRIVACY_RESTRICTED".into())
        )));
        for error in [
            ApiError::ButtonUrlInvalid,
            ApiError::Unknown("other privacy error".into()),
            ApiError::Unknown("BUTTON_USER_PRIVACY_RESTRICTED".into()),
            ApiError::Unknown("Bad Request: BUTTON_USER_PRIVACY_RESTRICTED!".into()),
        ] {
            assert!(!is_user_privacy_restriction(&RequestError::Api(error)));
        }
    }
}
