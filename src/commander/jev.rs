//! A small client for TypeSafe's System One endpoint, which serves Jev.
//!
//! Jev does not write text. It is sent some state and a set of typed questions,
//! and it answers each one with a probability: which option of a Choice, or how
//! likely a yes/no Noul is. Code decides what to do with the answers. Here the
//! state is the typed line and what herdr has on screen, and the questions are
//! built by [`super::plan`].
//!
//! The request is blocking and runs on a thread of its own, never on the UI
//! loop.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

const ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
const TIMEOUT: Duration = Duration::from_secs(20);

/// One typed question.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Question {
    /// Pick one option. Each option maps to a description of what it means.
    Choice {
        instructions: Value,
        criteria: BTreeMap<String, Value>,
    },
    /// How likely the answer is yes.
    Noul { instructions: Value },
}

impl Question {
    pub fn choice(instructions: impl Into<Value>, criteria: BTreeMap<String, Value>) -> Self {
        Question::Choice {
            instructions: instructions.into(),
            criteria,
        }
    }

    pub fn noul(instructions: impl Into<Value>) -> Self {
        Question::Noul {
            instructions: instructions.into(),
        }
    }
}

/// One answer. Only the fields herdr uses are read.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Answer {
    Choice {
        choice: String,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
    Noul {
        noul: f64,
    },
    Score {
        score: f64,
        confidence: f64,
    },
}

impl Answer {
    /// The chosen option and its probability, for a Choice.
    pub fn picked(&self) -> Option<(&str, f64)> {
        match self {
            Answer::Choice {
                choice,
                probabilities,
                ..
            } => Some((
                choice.as_str(),
                probabilities.get(choice).copied().unwrap_or(0.0),
            )),
            _ => None,
        }
    }

    /// Every option's probability, for a Choice.
    pub fn probabilities(&self) -> Option<&BTreeMap<String, f64>> {
        match self {
            Answer::Choice { probabilities, .. } => Some(probabilities),
            _ => None,
        }
    }

    /// The probability of yes, for a Noul.
    pub fn yes(&self) -> Option<f64> {
        match self {
            Answer::Noul { noul } => Some(*noul),
            _ => None,
        }
    }
}

pub type Questions = BTreeMap<String, Question>;
pub type Answers = BTreeMap<String, Answer>;

/// Something that answers questions about a state. The real one is [`Jev`];
/// tests answer from a script.
pub trait Oracle {
    fn ask(&self, state: &Value, questions: &Questions) -> Result<Answers, String>;
}

/// The TypeSafe API, reached with a key.
pub struct Jev {
    pub api_key: String,
    pub model: String,
}

#[derive(Serialize)]
struct Request<'a> {
    state: &'a Value,
    model: &'a str,
    questions: &'a Questions,
}

#[derive(Deserialize)]
struct Response {
    answers: Answers,
}

impl Oracle for Jev {
    fn ask(&self, state: &Value, questions: &Questions) -> Result<Answers, String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .build()
            .into();
        let body = Request {
            state,
            model: &self.model,
            questions,
        };
        let mut response = agent
            .post(ENDPOINT)
            .header("Authorization", &format!("Bearer {}", self.api_key))
            .send_json(&body)
            .map_err(|err| format!("could not reach Jev: {err}"))?;
        let status = response.status().as_u16();
        if status != 200 {
            let detail = response
                .body_mut()
                .read_to_string()
                .ok()
                .and_then(|text| error_message(&text))
                .unwrap_or_default();
            return Err(match status {
                401 | 403 => "Jev refused the API key — check it in settings".to_string(),
                429 => "Jev is rate limiting; try again in a moment".to_string(),
                _ if detail.is_empty() => format!("Jev answered {status}"),
                _ => format!("Jev answered {status}: {detail}"),
            });
        }
        let parsed: Response = response
            .body_mut()
            .read_json()
            .map_err(|err| format!("Jev's answer could not be read: {err}"))?;
        Ok(parsed.answers)
    }
}

/// The message out of an error body, if it has one.
fn error_message(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    let message = value
        .pointer("/error/message")
        .or_else(|| value.pointer("/detail/message"))
        .or_else(|| value.get("message"))
        .or_else(|| value.get("detail"))?;
    Some(match message {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn questions_serialize_the_way_the_api_reads_them() {
        let mut criteria = BTreeMap::new();
        criteria.insert("a".to_string(), json!("first"));
        let mut questions = Questions::new();
        questions.insert("pick".into(), Question::choice("which?", criteria));
        questions.insert("yes".into(), Question::noul("is it?"));
        let value = serde_json::to_value(&questions).unwrap();
        assert_eq!(
            value,
            json!({
                "pick": {"type": "choice", "instructions": "which?", "criteria": {"a": "first"}},
                "yes": {"type": "noul", "instructions": "is it?"}
            })
        );
    }

    /// Reaches the real endpoint. With `TYPESAFE_API_KEY` set it must answer;
    /// without it, a made-up key must be refused in words that say so.
    /// Run with `cargo test jev_live -- --ignored`.
    #[test]
    #[ignore = "reaches the network"]
    fn jev_live() {
        let key = std::env::var("TYPESAFE_API_KEY").ok();
        let jev = Jev {
            api_key: key.clone().unwrap_or_else(|| "not-a-key".into()),
            model: "jev-latest".into(),
        };
        let mut questions = Questions::new();
        questions.insert("urgent".into(), Question::noul("Does this convey urgency?"));
        let result = jev.ask(&json!("Help! The build is on fire."), &questions);
        match key {
            Some(_) => assert!(result.unwrap()["urgent"].yes().unwrap() > 0.5),
            None => assert!(result.unwrap_err().contains("API key")),
        }
    }

    #[test]
    fn answers_parse_from_the_documented_response() {
        let body = json!({
            "model": "jev-1.13.0",
            "answers": {
                "department": {
                    "type": "choice",
                    "choice": "billing",
                    "probabilities": {"billing": 0.88, "technical": 0.12},
                    "confidence": 0.81
                },
                "is_urgent": {"type": "noul", "noul": 0.95}
            },
            "usage": {"input_tokens": 318, "output_tokens": 34}
        });
        let parsed: Response = serde_json::from_value(body).unwrap();
        assert_eq!(
            parsed.answers["department"].picked(),
            Some(("billing", 0.88))
        );
        assert_eq!(parsed.answers["is_urgent"].yes(), Some(0.95));
    }
}
