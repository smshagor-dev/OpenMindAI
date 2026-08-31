#[derive(Debug, Default)]
pub(crate) struct Utf8StreamDecoder {
    pending: Vec<u8>,
}

impl Utf8StreamDecoder {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut output = String::new();

        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(valid) => {
                    output.push_str(valid);
                    self.pending.clear();
                    break;
                }
                Err(error) => {
                    let valid_up_to = error.valid_up_to();
                    if valid_up_to > 0 {
                        output.push_str(
                            std::str::from_utf8(&self.pending[..valid_up_to])
                                .expect("valid_up_to always points to valid UTF-8"),
                        );
                        self.pending.drain(..valid_up_to);
                    }

                    match error.error_len() {
                        Some(invalid_len) => {
                            output.push('\u{FFFD}');
                            self.pending.drain(..invalid_len.min(self.pending.len()));
                        }
                        None => break,
                    }
                }
            }
        }

        output
    }

    pub(crate) fn finish(&mut self) -> String {
        if self.pending.is_empty() {
            return String::new();
        }
        let output = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_ascii_immediately() {
        let mut decoder = Utf8StreamDecoder::default();
        assert_eq!(decoder.push(b"hello"), "hello");
        assert_eq!(decoder.finish(), "");
    }

    #[test]
    fn waits_for_split_multibyte_character() {
        let mut decoder = Utf8StreamDecoder::default();
        let euro = "€".as_bytes();
        assert_eq!(decoder.push(&euro[..1]), "");
        assert_eq!(decoder.push(&euro[1..]), "€");
        assert_eq!(decoder.finish(), "");
    }

    #[test]
    fn replaces_invalid_bytes_without_losing_following_text() {
        let mut decoder = Utf8StreamDecoder::default();
        assert_eq!(decoder.push(&[0xFF, b'o', b'k']), "�ok");
        assert_eq!(decoder.finish(), "");
    }
}
