//
// Copyright (C) 2024 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

use libsignal_core::Aci;
use serde_with::serde_as;

use crate::backup::serialize::{self, UnorderedList};
use crate::backup::{likely_empty, uuid_bytes_to_aci};
use crate::proto::backup as proto;

/// Validated version of [`proto::Text`].
#[derive(Debug, serde::Serialize)]
#[cfg_attr(test, derive(PartialEq))]
pub struct MessageText {
    pub text: String,
    pub ranges: UnorderedList<TextRange>,
}

#[derive(Debug, serde::Serialize)]
#[cfg_attr(test, derive(PartialEq, Clone))]
pub struct TextRange {
    pub start: u32,
    pub length: u32,
    pub effect: TextEffect,
}

#[serde_as]
#[derive(Debug, serde::Serialize)]
#[cfg_attr(test, derive(PartialEq, Clone))]
pub enum TextEffect {
    MentionAci(#[serde_as(as = "serialize::ServiceIdAsString")] Aci),
    Style(#[serde_as(as = "serialize::EnumAsString")] proto::body_range::Style),
}

#[derive(Debug, displaydoc::Display, thiserror::Error)]
#[cfg_attr(test, derive(PartialEq))]
pub enum TextError {
    /// body was empty
    EmptyBody,
    /// body was {0} bytes (too long)
    TooLongBody(usize),
    /// body was {0} bytes (too long to also have a long text attachment)
    TooLongBodyForLongText(usize),
    /// body was {0} bytes (too long to be in a quote)
    TooLongBodyForQuote(usize),
    /// mention had invalid ACI
    MentionInvalidAci,
    /// BodyRange.associatedValue is a oneof but has no value
    NoAssociatedValueForBodyRange,
}

const MAX_BODY_LENGTH: usize = 128 * 1024;
pub(crate) const MAX_BODY_LENGTH_WITH_LONG_TEXT_ATTACHMENT: usize = 2 * 1024;
pub(crate) const MAX_BODY_LENGTH_FOR_QUOTE: usize = 2 * 1024;

impl MessageText {
    pub fn check_length_with_long_text_attachment(&self) -> Result<(), TextError> {
        if self.text.len() > MAX_BODY_LENGTH_WITH_LONG_TEXT_ATTACHMENT {
            return Err(TextError::TooLongBodyForLongText(self.text.len()));
        }
        Ok(())
    }

    pub fn check_length_for_quote(&self) -> Result<(), TextError> {
        if self.text.len() > MAX_BODY_LENGTH_FOR_QUOTE {
            return Err(TextError::TooLongBodyForQuote(self.text.len()));
        }
        Ok(())
    }
}

impl TryFrom<proto::Text> for MessageText {
    type Error = TextError;

    fn try_from(value: proto::Text) -> Result<Self, Self::Error> {
        let proto::Text {
            body,
            bodyRanges,
            special_fields: _,
        } = value;

        match body.len() {
            0 => return Err(TextError::EmptyBody),
            1..=MAX_BODY_LENGTH => {}
            len => return Err(TextError::TooLongBody(len)),
        }

        let ranges = likely_empty(bodyRanges, |iter| {
            iter.map(|range| {
                let proto::BodyRange {
                    start,
                    length,
                    associatedValue,
                    special_fields: _,
                } = range;
                use proto::body_range::AssociatedValue;
                let effect =
                    match associatedValue.ok_or(TextError::NoAssociatedValueForBodyRange)? {
                        AssociatedValue::MentionAci(aci) => TextEffect::MentionAci(
                            uuid_bytes_to_aci(aci).map_err(|_| TextError::MentionInvalidAci)?,
                        ),
                        AssociatedValue::Style(style) => {
                            // All style values are valid
                            TextEffect::Style(style.enum_value_or_default())
                        }
                    };
                Ok(TextRange {
                    start,
                    length,
                    effect,
                })
            })
            .collect::<Result<_, _>>()
        })?;
        Ok(Self { text: body, ranges })
    }
}

#[cfg(test)]
mod test {
    use test_case::test_case;

    use super::*;
    use crate::backup::testutil::TEST_MESSAGE_TEXT;

    impl proto::Text {
        pub(crate) fn test_data() -> Self {
            Self {
                body: TEST_MESSAGE_TEXT.to_string(),
                bodyRanges: vec![proto::BodyRange {
                    start: 2,
                    length: 5,
                    associatedValue: Some(proto::body_range::AssociatedValue::Style(
                        proto::body_range::Style::MONOSPACE.into(),
                    )),
                    special_fields: Default::default(),
                }],
                special_fields: Default::default(),
            }
        }
    }

    impl MessageText {
        pub(crate) fn from_proto_test_data() -> Self {
            Self {
                text: TEST_MESSAGE_TEXT.to_string(),
                ranges: vec![TextRange {
                    start: 2,
                    length: 5,
                    effect: TextEffect::Style(proto::body_range::Style::MONOSPACE),
                }]
                .into(),
            }
        }
    }

    #[test]
    fn valid_text() {
        assert_eq!(
            proto::Text::test_data().try_into(),
            Ok(MessageText::from_proto_test_data())
        );
    }

    #[test_case(|x| x.body = "".into() => Err(TextError::EmptyBody); "empty body")]
    #[test_case(|x| x.body = "x".repeat(MAX_BODY_LENGTH) => Ok(()); "longest body")]
    #[test_case(|x| x.body = "x".repeat(MAX_BODY_LENGTH + 1) => Err(TextError::TooLongBody(MAX_BODY_LENGTH + 1)); "too long body")]
    #[test_case(|x| x.bodyRanges.push(Default::default()) => Err(TextError::NoAssociatedValueForBodyRange); "invalid body range")]
    #[test_case(|x| {
        x.bodyRanges.push(proto::BodyRange {
            associatedValue: Some(proto::body_range::AssociatedValue::MentionAci(vec![])),
            ..Default::default()
        });
    } => Err(TextError::MentionInvalidAci); "invalid mention")]
    #[test_case(|x| {
        x.bodyRanges.push(proto::BodyRange {
            associatedValue: Some(proto::body_range::AssociatedValue::MentionAci(proto::Contact::TEST_ACI.to_vec())),
            ..Default::default()
        });
    } => Ok(()); "valid mention")]
    fn text(modifier: fn(&mut proto::Text)) -> Result<(), TextError> {
        let mut message = proto::Text::test_data();
        modifier(&mut message);
        message.try_into().map(|_: MessageText| ())
    }

    #[test]
    fn ranges_are_sorted_when_serialized() {
        let range1 = TextRange {
            start: 2,
            length: 5,
            effect: TextEffect::Style(proto::body_range::Style::MONOSPACE),
        };
        let range2 = TextRange {
            start: 10,
            length: 2,
            effect: TextEffect::Style(proto::body_range::Style::BOLD),
        };

        let message1 = MessageText {
            ranges: vec![range1.clone(), range2.clone()].into(),
            ..MessageText::from_proto_test_data()
        };
        let message2 = MessageText {
            ranges: vec![range2, range1].into(),
            ..MessageText::from_proto_test_data()
        };

        assert_eq!(
            serde_json::to_string_pretty(&message1).expect("valid"),
            serde_json::to_string_pretty(&message2).expect("valid"),
        );
    }
}
