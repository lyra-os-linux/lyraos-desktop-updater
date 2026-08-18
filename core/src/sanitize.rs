pub const MAX_TECHNICAL_LINE_BYTES: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SanitizedLine {
    pub text: String,
    pub truncated: bool,
}

pub fn sanitize_technical_line(input: &str) -> SanitizedLine {
    let without_controls = strip_terminal_controls(input);
    let without_home = mask_home_paths(&without_controls);
    let without_url_secrets = mask_urls(&without_home);
    let masked = mask_assignments(&without_url_secrets);
    truncate_utf8(&masked, MAX_TECHNICAL_LINE_BYTES)
}

fn strip_terminal_controls(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            0x1b => {
                index += 1;
                if index >= bytes.len() {
                    break;
                }
                match bytes[index] {
                    b'[' => {
                        index += 1;
                        while index < bytes.len() {
                            let byte = bytes[index];
                            index += 1;
                            if (0x40..=0x7e).contains(&byte) {
                                break;
                            }
                        }
                    }
                    b']' => {
                        index += 1;
                        while index < bytes.len() {
                            if bytes[index] == 0x07 {
                                index += 1;
                                break;
                            }
                            if bytes[index] == 0x1b && bytes.get(index + 1).copied() == Some(b'\\')
                            {
                                index += 2;
                                break;
                            }
                            index += 1;
                        }
                    }
                    _ => index += 1,
                }
            }
            b'\t' => {
                output.push('\t');
                index += 1;
            }
            b'\n' => {
                output.push(' ');
                index += 1;
            }
            byte if byte < 0x20 || (0x7f..=0x9f).contains(&byte) => index += 1,
            _ => {
                let remainder = &input[index..];
                if let Some(character) = remainder.chars().next() {
                    output.push(character);
                    index += character.len_utf8();
                } else {
                    break;
                }
            }
        }
    }
    output
}

fn mask_home_paths(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut remainder = input;
    while let Some(position) = remainder.find("/home/") {
        output.push_str(&remainder[..position]);
        output.push_str("/home/<user>");
        let after_prefix = &remainder[position + "/home/".len()..];
        let user_end = after_prefix
            .find(|character: char| character == '/' || character.is_whitespace())
            .unwrap_or(after_prefix.len());
        remainder = &after_prefix[user_end..];
    }
    output.push_str(remainder);
    output
}

fn mask_urls(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    while cursor < input.len() {
        let http = input[cursor..].find("http://").map(|value| cursor + value);
        let https = input[cursor..].find("https://").map(|value| cursor + value);
        let Some(start) = [http, https].into_iter().flatten().min() else {
            output.push_str(&input[cursor..]);
            break;
        };
        output.push_str(&input[cursor..start]);
        let end = input[start..]
            .find(char::is_whitespace)
            .map(|value| start + value)
            .unwrap_or(input.len());
        output.push_str(&mask_url(&input[start..end]));
        cursor = end;
    }
    output
}

fn mask_url(url: &str) -> String {
    let scheme_end = url.find("://").map(|position| position + 3).unwrap_or(0);
    let path_start = url[scheme_end..]
        .find('/')
        .map(|position| scheme_end + position)
        .unwrap_or(url.len());
    let mut authority = url[scheme_end..path_start].to_string();
    if let Some(at) = authority.rfind('@') {
        authority.replace_range(..=at, "***@");
    }
    let tail = &url[path_start..];
    let sensitive_start = tail.find(['?', '#']).unwrap_or(tail.len());
    let mut result = format!(
        "{}{}{}",
        &url[..scheme_end],
        authority,
        &tail[..sensitive_start]
    );
    if sensitive_start < tail.len() {
        result.push_str("?***");
    }
    result
}

fn mask_assignments(input: &str) -> String {
    let mut result = input.to_string();
    for key in [
        "token=",
        "password=",
        "passwd=",
        "secret=",
        "apikey=",
        "api_key=",
    ] {
        let mut search_from = 0;
        loop {
            let lowercase = result[search_from..].to_ascii_lowercase();
            let Some(relative) = lowercase.find(key) else {
                break;
            };
            let value_start = search_from + relative + key.len();
            let value_end = result[value_start..]
                .find(|character: char| character.is_whitespace() || matches!(character, '&' | ';'))
                .map(|position| value_start + position)
                .unwrap_or(result.len());
            result.replace_range(value_start..value_end, "***");
            search_from = value_start + 3;
        }
    }
    result
}

fn truncate_utf8(input: &str, limit: usize) -> SanitizedLine {
    if input.len() <= limit {
        return SanitizedLine {
            text: input.to_string(),
            truncated: false,
        };
    }
    let mut end = limit;
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    SanitizedLine {
        text: input[..end].to_string(),
        truncated: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_terminal_escape_and_rewrite_controls() {
        let line = sanitize_technical_line(
            "ok\x1b[31mred\x1b[0m\rBAD\x08!\x1b]8;;https://evil\x07link\x1b]8;;\x07",
        );
        assert_eq!(line.text, "okredBAD!link");
    }

    #[test]
    fn masks_urls_assignments_and_home_paths() {
        let line = sanitize_technical_line(
            "https://user:pass@example.test/repo?token=abc#x /home/alice/file token=secret",
        );
        assert_eq!(
            line.text,
            "https://***@example.test/repo?*** /home/<user>/file token=***"
        );
    }

    #[test]
    fn truncates_on_utf8_boundary() {
        let line = sanitize_technical_line(&"á".repeat(3000));
        assert!(line.truncated);
        assert!(line.text.len() <= MAX_TECHNICAL_LINE_BYTES);
        assert!(line.text.is_char_boundary(line.text.len()));
    }
}
