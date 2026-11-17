//! The model's question card in the chat (`assist.ask`,
//! `docs/assistant-panel.md` §3.4): one compact card with the model's
//! questions, drawn with the workflow cards' controls
//! (`sparksmith_ui::chat_cards`). Recommended values are chosen when it
//! appears, so Enter through it accepts them. ↑↓ move over the current
//! question's options, Space chooses, Enter chooses and goes on (sending
//! after the last), Esc skips; a click chooses too. The person may answer
//! in their own words in the composer instead. Once answered the card is a
//! one-line summary.
use serde_json::{Map, Number, Value};
use sparksmith_automation::workflow::ask::{AskParams, Kind, MAX_ANSWER, Question};
use sparksmith_ui::chat_cards as cards;

/// The card's heading when the model gives none.
const DEFAULT_TITLE: &str = "A few questions";

/// One question and what the person gave so far.
#[derive(Clone, Debug, PartialEq)]
pub struct Asked {
    pub question: Question,
    pub chosen: Vec<String>,
    pub text: String,
    pub error: Option<String>,
}

/// How a card was answered.
#[derive(Clone, Debug, PartialEq)]
pub enum Answered {
    /// On the card: the summary line ("Power: USB · Where: Outdoors").
    Card(String),
    /// In the person's own words in the composer.
    Said(String),
    /// Skipped: left to the assistant.
    Skipped,
    /// The turn stopped before an answer.
    Stopped,
}

/// The card in the transcript.
#[derive(Clone, Debug, PartialEq)]
pub struct QuestionCard {
    /// Its id, as `assist.ask` answered it.
    pub card: String,
    pub title: String,
    pub why: String,
    pub questions: Vec<Asked>,
    /// The question the keys are on, and the option lit in it.
    pub current: usize,
    pub lit: usize,
    pub answer: Option<Answered>,
}

/// What the person did on the card.
#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    /// Send: the answers by question id.
    Send(Map<String, Value>),
    Skip,
}

impl QuestionCard {
    /// The card `card` for `params`, its recommended values chosen.
    pub fn new(card: String, params: &AskParams) -> Self {
        let questions = params
            .questions
            .iter()
            .map(|question| {
                let (chosen, text) = match &question.kind {
                    Kind::Single { recommended, .. } => {
                        (recommended.iter().cloned().collect(), String::new())
                    }
                    Kind::Multiple { recommended, .. } => (recommended.clone(), String::new()),
                    Kind::Number { recommended, .. } => (
                        Vec::new(),
                        recommended
                            .as_ref()
                            .map(Number::to_string)
                            .unwrap_or_default(),
                    ),
                    Kind::Text { recommended } => {
                        (Vec::new(), recommended.clone().unwrap_or_default())
                    }
                };
                Asked {
                    question: question.clone(),
                    chosen,
                    text,
                    error: None,
                }
            })
            .collect();
        let mut made = Self {
            card,
            title: params
                .title
                .clone()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_TITLE.to_owned()),
            why: params.why.clone().unwrap_or_default(),
            questions,
            current: 0,
            lit: 0,
            answer: None,
        };
        made.lit = made.first_lit(0);
        made
    }

    /// The option lit when the keys come to question `index`: its first
    /// chosen one, or the first.
    fn first_lit(&self, index: usize) -> usize {
        self.questions
            .get(index)
            .and_then(|asked| {
                options(&asked.question)?
                    .iter()
                    .position(|o| asked.chosen.contains(&o.value))
            })
            .unwrap_or(0)
    }

    /// Choose option `row` of question `index`: a single choice replaces,
    /// a multiple one toggles.
    fn choose(&mut self, index: usize, row: usize) {
        let Some(asked) = self.questions.get_mut(index) else {
            return;
        };
        let (multiple, value) = match &asked.question.kind {
            Kind::Single { options, .. } => (false, options.get(row)),
            Kind::Multiple { options, .. } => (true, options.get(row)),
            _ => return,
        };
        let Some(value) = value.map(|o| o.value.clone()) else {
            return;
        };
        asked.error = None;
        if !multiple {
            asked.chosen = vec![value];
        } else if asked.chosen.contains(&value) {
            asked.chosen.retain(|c| *c != value);
        } else {
            asked.chosen.push(value);
        }
    }

    /// The answers by question id, or `None` with each problem marked on
    /// its question.
    pub fn answers(&mut self) -> Option<Map<String, Value>> {
        let mut answers = Map::new();
        let mut ok = true;
        for asked in &mut self.questions {
            match value(asked) {
                Ok(value) => {
                    asked.error = None;
                    answers.insert(asked.question.id.clone(), value);
                }
                Err(why) => {
                    asked.error = Some(why);
                    ok = false;
                }
            }
        }
        ok.then_some(answers)
    }

    /// The answered card's line: each question's title and answer.
    pub fn summary(&self) -> String {
        self.questions
            .iter()
            .filter_map(|asked| {
                let shown = shown(asked);
                (!shown.is_empty()).then(|| format!("{}: {shown}", asked.question.title))
            })
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

/// A choice question's options.
fn options(question: &Question) -> Option<&[sparksmith_automation::workflow::ask::AskOption]> {
    match &question.kind {
        Kind::Single { options, .. } | Kind::Multiple { options, .. } => Some(options),
        _ => None,
    }
}

/// What the person gave for `asked`, as its answer.
fn value(asked: &Asked) -> Result<Value, String> {
    let text = asked.text.trim();
    match &asked.question.kind {
        Kind::Single { .. } => asked
            .chosen
            .first()
            .map(|v| Value::String(v.clone()))
            .ok_or_else(|| "choose one".to_owned()),
        Kind::Multiple { .. } => Ok(Value::Array(
            asked.chosen.iter().cloned().map(Value::String).collect(),
        )),
        Kind::Number { min, max, unit, .. } => {
            let word = text.split_whitespace().next().unwrap_or_default();
            let n: f64 = word.parse().map_err(|_| "give a number".to_owned())?;
            if !n.is_finite() {
                return Err("give a number".into());
            }
            let unit = if unit.is_empty() {
                String::new()
            } else {
                format!(" {unit}")
            };
            if let Some(least) = min.as_ref().and_then(Number::as_f64)
                && n < least
            {
                return Err(format!("at least {least}{unit}"));
            }
            if let Some(most) = max.as_ref().and_then(Number::as_f64)
                && n > most
            {
                return Err(format!("at most {most}{unit}"));
            }
            let whole = n.fract() == 0.0 && n.abs() < 9.0e15;
            let number = if whole {
                Some(Number::from(n as i64))
            } else {
                Number::from_f64(n)
            };
            number
                .map(Value::Number)
                .ok_or_else(|| "give a number".into())
        }
        Kind::Text { .. } => {
            if text.is_empty() {
                Err("write a few words".into())
            } else if text.len() > MAX_ANSWER {
                Err(format!("at most {MAX_ANSWER} characters"))
            } else {
                Ok(Value::String(text.to_owned()))
            }
        }
    }
}

/// `asked`'s answer as the summary shows it.
fn shown(asked: &Asked) -> String {
    match &asked.question.kind {
        Kind::Single { options, .. } | Kind::Multiple { options, .. } => options
            .iter()
            .filter(|o| asked.chosen.contains(&o.value))
            .map(|o| o.title.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        Kind::Number { unit, .. } => {
            let text = asked.text.trim();
            if text.is_empty() || unit.is_empty() || text.ends_with(unit.as_str()) {
                text.to_owned()
            } else {
                format!("{text} {unit}")
            }
        }
        Kind::Text { .. } => asked.text.trim().to_owned(),
    }
}

/// The field id of question `index` on `card`.
fn field_id(card: &str, index: usize) -> egui::Id {
    egui::Id::unique(("assistant-question", card, index))
}

/// Draw `card`; answers what the person chose. `active`: the card the turn
/// waits on.
pub fn show(ui: &mut egui::Ui, card: &mut QuestionCard, active: bool) -> Option<Choice> {
    if let Some(answer) = &card.answer {
        let (how, line) = match answer {
            Answered::Card(line) => ("Answered", line.clone()),
            Answered::Said(text) => ("Answered in the chat", text.clone()),
            Answered::Skipped => ("Skipped", "Left to the assistant".to_owned()),
            Answered::Stopped => ("Stopped", String::new()),
        };
        cards::card(ui, &card.title, how, |ui| {
            if !line.is_empty() {
                cards::note(ui, &line);
            }
        });
        return None;
    }
    let ctx = ui.ctx().clone();
    let fields: Vec<egui::Id> = (0..card.questions.len())
        .map(|i| field_id(&card.card, i))
        .collect();
    let first_wait_key = egui::Id::unique(("assistant-question-shown", &card.card));
    let first_wait = active && !ctx.data(|d| d.get_temp::<bool>(first_wait_key).unwrap_or(false));
    if first_wait {
        ctx.data_mut(|d| d.insert_temp(first_wait_key, true));
        focus_current(&ctx, card, &fields);
    }
    let focused = ctx.memory(|m| m.focused());
    let mut choice = None;
    let mut advance = false;
    if active {
        match focused {
            None => {
                let nav = cards::QuestionNav::take(ui);
                if nav.escape {
                    choice = Some(Choice::Skip);
                }
                let count = card
                    .questions
                    .get(card.current)
                    .and_then(|a| options(&a.question))
                    .map_or(0, <[_]>::len);
                if count > 0 {
                    if nav.up {
                        card.lit = card.lit.saturating_sub(1);
                    }
                    if nav.down {
                        card.lit = (card.lit + 1).min(count - 1);
                    }
                    let single = card
                        .questions
                        .get(card.current)
                        .is_some_and(|a| matches!(a.question.kind, Kind::Single { .. }));
                    if nav.space || (nav.enter && single) {
                        card.choose(card.current, card.lit);
                    }
                }
                advance |= nav.enter;
            }
            Some(id) if fields.contains(&id) => {
                if cards::QuestionNav::escape(ui) {
                    choice = Some(Choice::Skip);
                }
            }
            // The composer, or another field: the person types elsewhere.
            Some(_) => {}
        }
    }
    let mut send = false;
    let title = card.title.clone();
    cards::card(ui, &title, "", |ui| {
        if !card.why.is_empty() {
            cards::note(ui, &card.why);
        }
        let current = card.current;
        let lit = card.lit;
        let mut clicked = None;
        for (index, asked) in card.questions.iter_mut().enumerate() {
            let id = fields[index];
            let error = asked.error.clone();
            let invalid = error.is_some();
            let Asked {
                question,
                chosen,
                text,
                error: own_error,
            } = asked;
            cards::compact_row(ui, &question.title, error.as_deref(), |ui| {
                ui.vertical(|ui| {
                    if let Some(words) = question.text.as_deref().filter(|w| !w.is_empty()) {
                        cards::note(ui, words);
                    }
                    let response = match &question.kind {
                        Kind::Single { options, .. } | Kind::Multiple { options, .. } => {
                            let multiple = matches!(question.kind, Kind::Multiple { .. });
                            let recommended_many: Vec<String> = match &question.kind {
                                Kind::Multiple { recommended, .. } => recommended.clone(),
                                Kind::Single { recommended, .. } => {
                                    recommended.iter().cloned().collect()
                                }
                                _ => Vec::new(),
                            };
                            let rows: Vec<cards::QuestionOption<'_>> = options
                                .iter()
                                .map(|o| cards::QuestionOption {
                                    value: &o.value,
                                    title: &o.title,
                                    about: &o.about,
                                    recommended: recommended_many.contains(&o.value),
                                })
                                .collect();
                            let lit = (active && index == current).then_some(lit);
                            if let Some(row) =
                                cards::question_options(ui, &rows, chosen, multiple, lit)
                            {
                                clicked = Some((index, row));
                            }
                            None
                        }
                        Kind::Number { unit, .. } => {
                            Some(cards::number_input(ui, id, text, unit, invalid))
                        }
                        Kind::Text { .. } => {
                            Some(cards::text_input(ui, id, text, false, MAX_ANSWER, invalid))
                        }
                    };
                    if let Some(response) = response {
                        if response.changed() {
                            *own_error = None;
                        }
                        if response.gained_focus() {
                            clicked = Some((index, usize::MAX));
                        }
                        if active && cards::entered(ui, &response) {
                            advance = true;
                        }
                    }
                });
            });
        }
        if let Some((index, row)) = clicked {
            card.current = index;
            if row != usize::MAX {
                card.lit = row;
                card.choose(index, row);
            }
        }
        match cards::footer(
            ui,
            &[
                cards::FooterButton::new("Skip"),
                cards::FooterButton::primary("Answer"),
            ],
        ) {
            Some(0) => choice = Some(Choice::Skip),
            Some(1) => send = true,
            _ => {}
        }
    });
    if advance && choice.is_none() {
        if card.current + 1 < card.questions.len() {
            card.current += 1;
            card.lit = card.first_lit(card.current);
            focus_current(&ctx, card, &fields);
        } else {
            send = true;
        }
    }
    if send && choice.is_none() {
        match card.answers() {
            Some(answers) => {
                card.answer = Some(Answered::Card(card.summary()));
                choice = Some(Choice::Send(answers));
            }
            // Go to the first question to fix.
            None => {
                if let Some(bad) = card.questions.iter().position(|a| a.error.is_some()) {
                    card.current = bad;
                    card.lit = card.first_lit(bad);
                    focus_current(&ctx, card, &fields);
                }
            }
        }
    }
    if choice == Some(Choice::Skip) {
        card.answer = Some(Answered::Skipped);
    }
    choice
}

/// Give the keyboard to the current question: its field, or else the card
/// itself (out of the composer), so the keys reach its options.
fn focus_current(ctx: &egui::Context, card: &QuestionCard, fields: &[egui::Id]) {
    let typed = card
        .questions
        .get(card.current)
        .is_some_and(|a| options(&a.question).is_none());
    match fields.get(card.current) {
        Some(id) if typed => ctx.memory_mut(|m| m.request_focus(*id)),
        _ => {
            if let Some(id) = ctx.memory(|m| m.focused()) {
                ctx.memory_mut(|m| m.surrender_focus(id));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn card() -> Result<QuestionCard, serde_json::Error> {
        let params: AskParams = serde_json::from_value(json!({"questions": [
            {"id": "power", "title": "Power", "type": "single",
             "options": [{"value": "usb", "title": "USB"}, {"value": "battery", "title": "Battery"}],
             "recommended": "usb"},
            {"id": "place", "title": "Where", "type": "multiple",
             "options": [{"value": "in", "title": "Indoors"}, {"value": "out", "title": "Outdoors"}],
             "recommended": ["out"]},
            {"id": "every", "title": "How often", "type": "number", "unit": "minutes",
             "min": 1, "max": 60, "recommended": 10},
            {"id": "name", "title": "Name", "type": "text"}
        ]}))?;
        Ok(QuestionCard::new("card-1".into(), &params))
    }

    #[test]
    fn recommended_values_are_chosen_and_answers_read_back() -> Result<(), serde_json::Error> {
        let mut card = card()?;
        assert_eq!(card.title, DEFAULT_TITLE);
        // The text question has no recommendation: it must be filled.
        assert!(card.answers().is_none());
        assert_eq!(
            card.questions[3].error.as_deref(),
            Some("write a few words")
        );
        card.questions[3].text = "Greenhouse".into();
        card.choose(1, 0);
        let answers = card.answers();
        assert_eq!(
            answers.map(Value::Object),
            Some(
                json!({"power": "usb", "place": ["out", "in"], "every": 10, "name": "Greenhouse"})
            )
        );
        assert_eq!(
            card.summary(),
            "Power: USB · Where: Indoors, Outdoors · How often: 10 minutes · Name: Greenhouse"
        );
        Ok(())
    }

    #[test]
    fn a_number_outside_its_range_is_marked() -> Result<(), serde_json::Error> {
        let mut card = card()?;
        card.questions[2].text = "90".into();
        card.questions[3].text = "x".into();
        assert!(card.answers().is_none());
        assert_eq!(
            card.questions[2].error.as_deref(),
            Some("at most 60 minutes")
        );
        card.questions[2].text = "2.5 minutes".into();
        assert_eq!(
            card.answers().and_then(|a| a.get("every").cloned()),
            Some(json!(2.5))
        );
        Ok(())
    }
}
