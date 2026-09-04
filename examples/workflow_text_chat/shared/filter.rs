use std::io::{self, Write};

/// Incrementally hides `<think>...</think>` without leaking partial tags.
pub struct ThinkingFilter {
    show_thinking: bool,
    inside_think: bool,
    buffer: String,
    visible: String,
}

impl ThinkingFilter {
    pub fn new(show_thinking: bool) -> Self {
        Self {
            show_thinking,
            inside_think: false,
            buffer: String::new(),
            visible: String::new(),
        }
    }

    pub fn write(&mut self, text: &str) -> io::Result<()> {
        if self.show_thinking {
            print_flush(text)?;
            self.visible.push_str(text);
            return Ok(());
        }
        self.buffer.push_str(text);
        loop {
            if self.inside_think {
                let Some(index) = self.buffer.find("</think>") else {
                    retain_possible_tag_prefix(&mut self.buffer, "</think>");
                    return Ok(());
                };
                self.buffer.drain(..index + "</think>".len());
                self.inside_think = false;
                continue;
            }
            if let Some(index) = self.buffer.find("<think>") {
                let visible = self.buffer[..index].to_owned();
                print_flush(&visible)?;
                self.visible.push_str(&visible);
                self.buffer.drain(..index + "<think>".len());
                self.inside_think = true;
                continue;
            }
            let mut safe = self.buffer.len().saturating_sub(6);
            while safe > 0 && !self.buffer.is_char_boundary(safe) {
                safe -= 1;
            }
            if safe > 0 {
                self.flush_prefix(safe)?;
            }
            return Ok(());
        }
    }

    fn flush_prefix(&mut self, length: usize) -> io::Result<()> {
        let visible = self.buffer[..length].to_owned();
        print_flush(&visible)?;
        self.visible.push_str(&visible);
        self.buffer.drain(..length);
        Ok(())
    }

    pub fn finish(mut self) -> io::Result<String> {
        if !self.show_thinking && !self.inside_think && !self.buffer.is_empty() {
            let rest = std::mem::take(&mut self.buffer);
            print_flush(&rest)?;
            self.visible.push_str(&rest);
        }
        Ok(self.visible)
    }
}

fn retain_possible_tag_prefix(buffer: &mut String, tag: &str) {
    let keep = (1..tag.len())
        .rev()
        .find(|length| buffer.ends_with(&tag[..*length]))
        .unwrap_or(0);
    if keep == 0 {
        buffer.clear();
    } else {
        buffer.drain(..buffer.len() - keep);
    }
}

fn print_flush(value: &str) -> io::Result<()> {
    print!("{value}");
    io::stdout().flush()
}

pub fn strip_thinking(value: &str) -> String {
    let mut output = String::new();
    let mut rest = value;
    loop {
        let Some(start) = rest.find("<think>") else {
            output.push_str(rest);
            break;
        };
        output.push_str(&rest[..start]);
        let after = &rest[start + "<think>".len()..];
        let Some(end) = after.find("</think>") else {
            break;
        };
        rest = &after[end + "</think>".len()..];
    }
    output.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_tags_split_across_chunks() {
        let mut filter = ThinkingFilter::new(false);
        filter.write("Before <thi").unwrap();
        filter.write("nk>private</thi").unwrap();
        filter.write("nk> after").unwrap();
        assert_eq!(filter.finish().unwrap(), "Before  after");
    }
}
