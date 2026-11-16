use futures_util::StreamExt;
use reqwest::Response;

/// Splits a byte stream into lines without decoding chunks on their own, so UTF-8
/// sequences split across network reads stay whole.
#[derive(Default)]
pub struct LineDecoder {
    buf: Vec<u8>,
}

/// What a line handler wants next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    More,
    Done,
}

impl LineDecoder {
    pub fn push(
        &mut self,
        chunk: &[u8],
        on_line: &mut impl FnMut(&str) -> Result<Flow, String>,
    ) -> Result<Flow, String> {
        self.buf.extend_from_slice(chunk);
        let mut consumed = 0;
        let mut flow = Flow::More;
        while let Some(pos) = self.buf[consumed..].iter().position(|&b| b == b'\n') {
            let end = consumed + pos;
            let line = String::from_utf8_lossy(&self.buf[consumed..end]);
            consumed = end + 1;
            let line = line.trim();
            if !line.is_empty() && on_line(line)? == Flow::Done {
                flow = Flow::Done;
                break;
            }
        }
        self.buf.drain(..consumed);
        Ok(flow)
    }

    pub fn finish(self, on_line: &mut impl FnMut(&str) -> Result<Flow, String>) -> Result<(), String> {
        let rest = String::from_utf8_lossy(&self.buf);
        let rest = rest.trim();
        if !rest.is_empty() {
            on_line(rest)?;
        }
        Ok(())
    }
}

/// Feeds every non-empty line of a streamed response to `on_line` until it returns `Flow::Done`.
pub async fn for_each_line(res: Response, mut on_line: impl FnMut(&str) -> Result<Flow, String>) -> Result<(), String> {
    let mut stream = res.bytes_stream();
    let mut decoder = LineDecoder::default();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Failed to read stream chunk: {}", e))?;
        if decoder.push(&chunk, &mut on_line)? == Flow::Done {
            return Ok(());
        }
    }
    decoder.finish(&mut on_line)
}

/// The JSON payload of an SSE `data:` line; `None` for comments, event names and blank keep-alives.
pub fn data_payload(line: &str) -> Option<&str> {
    line.strip_prefix("data:").map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_multibyte_characters_split_across_chunks() {
        let text = "data: héllo\n";
        let bytes = text.as_bytes();
        let split = text.find('é').unwrap() + 1;
        let mut lines = Vec::new();
        let mut collect = |l: &str| {
            lines.push(l.to_string());
            Ok(Flow::More)
        };
        let mut decoder = LineDecoder::default();
        decoder.push(&bytes[..split], &mut collect).unwrap();
        decoder.push(&bytes[split..], &mut collect).unwrap();
        decoder.finish(&mut collect).unwrap();
        assert_eq!(lines, vec!["data: héllo"]);
    }

    #[test]
    fn stops_at_done_and_flushes_unterminated_tail() {
        let mut lines = Vec::new();
        let mut collect = |l: &str| {
            lines.push(l.to_string());
            Ok(if l == "stop" { Flow::Done } else { Flow::More })
        };
        let mut decoder = LineDecoder::default();
        assert_eq!(decoder.push(b"a\n\nstop\nignored\n", &mut collect).unwrap(), Flow::Done);
        let mut tail = LineDecoder::default();
        tail.push(b"x\ny", &mut collect).unwrap();
        tail.finish(&mut collect).unwrap();
        assert_eq!(lines, vec!["a", "stop", "x", "y"]);
    }
}
