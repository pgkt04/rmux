// Ported from tmux control.c @ 8f25579c
#[derive(Debug, Default)]
pub struct ControlInput {
    bytes: Vec<u8>,
    pub exited: bool,
}
#[derive(Debug, PartialEq, Eq)]
pub enum InputLine {
    Command(Vec<u8>),
    Exit,
}
impl ControlInput {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<InputLine> {
        if self.exited {
            return Vec::new();
        }
        self.bytes.extend_from_slice(bytes);
        let mut result = Vec::new();
        let mut consumed = 0;
        while let Some(n) = self.bytes[consumed..].iter().position(|b| *b == b'\n') {
            let physical = &self.bytes[consumed..consumed + n];
            let line = super::cstring(physical);
            consumed += n + 1;
            if line.is_empty() {
                self.exited = true;
                result.push(InputLine::Exit);
                break;
            }
            result.push(InputLine::Command(line.to_vec()));
        }
        self.bytes.drain(..consumed);
        result
    }
    pub fn eof(&mut self) {
        self.exited = true;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragments_nuls_and_empty_stop() {
        let mut i = ControlInput::default();
        assert!(i.feed(b"dis").is_empty());
        assert_eq!(
            i.feed(b"play\0ignored\n\nignored\n"),
            vec![InputLine::Command(b"display".to_vec()), InputLine::Exit]
        );
        assert!(i.feed(b"more\n").is_empty());
        let mut i = ControlInput::default();
        assert_eq!(
            i.feed(b"cmd\r\n"),
            vec![InputLine::Command(b"cmd\r".to_vec())]
        );
        assert!(i.feed(b"fragment").is_empty());
        i.eof();
        assert!(i.feed(b"\n").is_empty());
    }
}
