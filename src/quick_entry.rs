pub fn parse(text: &str, explicit_due: Option<u16>) -> Result<(String, Option<u16>), String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("先写下任务名称".into());
    }
    let split = text.find(char::is_whitespace).unwrap_or(text.len());
    let first = &text[..split];
    if let Some((hour, minute)) = first.split_once(':')
        && !hour.is_empty()
        && hour.len() <= 2
        && hour.bytes().all(|b| b.is_ascii_digit())
        && minute.len() == 2
        && minute.bytes().all(|b| b.is_ascii_digit())
    {
        let hour = hour.parse::<u16>().unwrap_or(24);
        let minute = minute.parse::<u16>().unwrap_or(60);
        if hour >= 24 || minute >= 60 {
            return Err("时间请写为 00:00 至 23:59".into());
        }
        let title = text[split..].trim();
        if title.is_empty() {
            return Err("请在时间后写下任务名称，例如 15:30 发送周报".into());
        }
        return Ok((title.into(), explicit_due.or(Some(hour * 60 + minute))));
    }
    Ok((text.into(), explicit_due))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_time_prefix_saves_a_separate_deadline_without_retyping_the_title() {
        assert_eq!(
            parse("15:30 发送周报", None).unwrap(),
            ("发送周报".into(), Some(930))
        );
        assert_eq!(
            parse("9:05 回邮件", None).unwrap(),
            ("回邮件".into(), Some(545))
        );
        assert_eq!(
            parse("15:30 发送周报", Some(1020)).unwrap(),
            ("发送周报".into(), Some(1020))
        );
    }
    #[test]
    fn ordinary_text_and_unknown_dates_are_never_silently_interpreted() {
        assert_eq!(
            parse("复习 15:30 的录像", None).unwrap(),
            ("复习 15:30 的录像".into(), None)
        );
        assert_eq!(parse("明天开会", None).unwrap(), ("明天开会".into(), None));
        assert!(parse("25:30 发送周报", None).is_err());
        assert!(parse("15:30", None).is_err());
    }
}
