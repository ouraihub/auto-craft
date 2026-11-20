//! Reference: craft-anything ai/src/rebuild/feed.rs::apply (300dba58).
//! Native session snapshot/delta folding without importing historic turns twice.
use super::*;
impl Continuity {
    fn words(&mut self, key: &str, text: &str) -> Result<()> {
        let previous = self
            .text
            .insert(key.into(), text.into())
            .unwrap_or_default();
        let delta = text.strip_prefix(&previous).unwrap_or(text);
        if delta.is_empty() {
            return Ok(());
        }
        match self.items.last_mut() {
            Some(FeedItem::Agent {
                text,
                streaming: true,
            }) => text.push_str(delta),
            _ => self.items.push(FeedItem::Agent {
                text: delta.into(),
                streaming: true,
            }),
        }
        self.activity.record(LineKind::Said, delta)
    }
    fn message(&mut self, message: &Message) -> Result<()> {
        if message.role != Role::Assistant {
            return Ok(());
        }
        for (index, block) in message.content.iter().enumerate() {
            match block {
                ContentBlock::Text { text } => {
                    self.words(&format!("{}:{index}", message.id), text)?
                }
                ContentBlock::Tool {
                    tool_call_id,
                    name,
                    status,
                    input,
                    output,
                } => {
                    if self.tools.get(tool_call_id) == Some(status) {
                        continue;
                    }
                    let first = self.tools.insert(tool_call_id.clone(), *status).is_none();
                    let name = name.rsplit("__").next().unwrap_or(name);
                    if first {
                        self.end_stream();
                        match self.items.last_mut() {
                            Some(FeedItem::Work { tools, .. }) => tools.push((name.into(), None)),
                            _ => self.items.push(FeedItem::Work {
                                tools: vec![(name.into(), None)],
                                notes: vec![],
                            }),
                        }
                        let subject = input
                            .as_ref()
                            .and_then(|input| {
                                [
                                    "command",
                                    "path",
                                    "file_path",
                                    "file",
                                    "url",
                                    "target",
                                    "program",
                                    "query",
                                    "output_directory",
                                ]
                                .into_iter()
                                .find_map(|field| input.get(field).and_then(Value::as_str))
                            })
                            .unwrap_or("");
                        self.activity.record(
                            LineKind::Started,
                            &format!("{name} {}", crate::activity::object(subject)),
                        )?;
                    }
                    match status {
                        ToolStatus::Running => {}
                        ToolStatus::Completed | ToolStatus::Failed => {
                            let ok = *status == ToolStatus::Completed;
                            for item in self.items.iter_mut().rev() {
                                if let FeedItem::Work { tools, .. } = item
                                    && let Some(slot) = tools
                                        .iter_mut()
                                        .rev()
                                        .find(|(n, done)| n == name && done.is_none())
                                {
                                    slot.1 = Some(ok);
                                    break;
                                }
                            }
                            self.activity.record(
                                if ok { LineKind::Done } else { LineKind::Failed },
                                &crate::activity::object(name),
                            )?;
                        }
                    }
                    if matches!(status, ToolStatus::Completed | ToolStatus::Failed)
                        && let Some(output) = output
                    {
                        self.activity.record(
                            LineKind::Output,
                            &output.chars().take(600).collect::<String>(),
                        )?;
                    }
                }
                ContentBlock::Thinking { .. } | ContentBlock::Image { .. } => {}
            }
        }
        Ok(())
    }
    pub(super) fn agent(&mut self, event: &SessionEvent) -> Result<()> {
        match event {
            SessionEvent::Message { message, .. } => self.message(message),
            SessionEvent::Snapshot { snapshot } => {
                if !std::mem::replace(&mut self.seeded, true) {
                    for message in &snapshot.messages {
                        if message.role == Role::Assistant {
                            for (index, block) in message.content.iter().enumerate() {
                                match block {
                                    ContentBlock::Text { text } => {
                                        self.text.insert(
                                            format!("{}:{index}", message.id),
                                            text.clone(),
                                        );
                                    }
                                    ContentBlock::Tool {
                                        tool_call_id,
                                        status,
                                        ..
                                    } => {
                                        self.tools.insert(tool_call_id.clone(), *status);
                                    }
                                    ContentBlock::Thinking { .. } | ContentBlock::Image { .. } => {}
                                }
                            }
                        }
                    }
                } else {
                    for message in &snapshot.messages {
                        self.message(message)?;
                    }
                }
                if snapshot.phase == SessionPhase::Idle {
                    self.end_stream();
                }
                Ok(())
            }
            SessionEvent::Delta {
                message_id,
                block,
                text,
                ..
            } => {
                let key = format!("{message_id}:{block}");
                if let Some(previous) = self.text.get(&key) {
                    let mut total = previous.clone();
                    total.push_str(text);
                    self.words(&key, &total)?;
                }
                Ok(())
            }
            SessionEvent::Error { error, .. } => {
                self.end_stream();
                self.items.push(FeedItem::Error {
                    message: error.message.clone(),
                    remediation: None,
                    log: None,
                });
                self.activity
                    .record(LineKind::Problem, "The builder reported an error.")
            }
        }
    }
}
