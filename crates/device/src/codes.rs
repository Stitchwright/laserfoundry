//! What GRBL's numbered replies mean, in words. The numbers are from the public GRBL 1.1 interface
//! documentation (<https://github.com/gnea/grbl/wiki/Grbl-v1.1-Interface#grbl-response-messages> and
//! <https://github.com/gnea/grbl/wiki/Grbl-v1.1-Interface#alarm-codes>); other firmware may use them differently,
//! so the words are a hint and the number is always shown too.

/// What `error:<code>` means.
pub fn error_text(code: u32) -> Option<&'static str> {
    Some(match code {
        1 => "a G-code word has no command letter",
        2 => "a number in the line has a bad format",
        3 => "an unsupported `$` setting command",
        4 => "a setting value is negative",
        5 => "homing is not enabled ($22)",
        6 => "a setting value is below the minimum step pulse",
        7 => "the settings could not be read from memory",
        8 => "that command needs the machine to be idle",
        9 => "G-code is locked out (an alarm or the machine is not unlocked): use Unlock ($X) or Home ($H)",
        10 => "soft limits need homing to be enabled",
        11 => "the line is too long for the controller",
        12 => "a setting would exceed the maximum step rate",
        13 => "the safety door is open",
        14 => "a line is too long to be stored in the controller's memory",
        15 => "the jog target is outside the machine's travel",
        16 => "a jog command is malformed",
        17 => "laser mode needs a PWM-capable spindle output",
        20 => "an unsupported or invalid G-code command",
        21 => "two G-code commands from the same group are in one line",
        22 => "no feed rate has been set",
        23 => "a G-code word needs a whole number",
        24 => "two commands in one line both want the axis words",
        25 => "a G-code word is repeated in the line",
        26 => "a command needs axis words and has none",
        27 => "the line number is invalid",
        28 => "a command needs a value word and has none",
        29 => "the controller does not support this work coordinate system",
        30 => "a G53 command needs G0 or G1 motion",
        31 => "axis words were given that no command uses",
        32 => "an arc command needs axis words in the current plane",
        33 => "the motion target is invalid",
        34 => "an arc's radius is invalid",
        35 => "an arc command needs offset words in the current plane",
        36 => "a line has words that nothing uses",
        37 => "a tool length offset is not assigned to the configured tool axis",
        38 => "the tool number is larger than the controller supports",
        _ => return None,
    })
}

/// What `ALARM:<code>` means.
pub fn alarm_text(code: u32) -> Option<&'static str> {
    Some(match code {
        1 => "a hard limit switch was hit; the position may be lost",
        2 => "the motion would go past the soft limits",
        3 => "the machine was reset while moving, so its position may be lost: home it or check where the head is",
        4 => "the probe failed (it was not in the expected state)",
        5 => "the probe did not make contact",
        6 => "homing failed: the cycle was reset",
        7 => "homing failed: the safety door was opened",
        8 => "homing failed: the switch did not clear when pulling off",
        9 => "homing failed: no limit switch was found",
        _ => return None,
    })
}

/// `(…)` for an error code, for a message that puts a space between the number and this; `(unknown code)` when
/// the code is not known (a message with the place always filled can be translated by template).
pub fn error_hint(code: u32) -> String {
    format!("({})", error_text(code).unwrap_or("unknown code"))
}

/// The same for an alarm code.
pub fn alarm_hint(code: u32) -> String {
    format!("({})", alarm_text(code).unwrap_or("unknown code"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_codes_have_words_and_unknown_ones_stay_numbers() {
        assert!(error_text(9).unwrap().contains("Unlock"));
        assert!(error_text(20).unwrap().contains("unsupported"));
        assert_eq!(error_text(0), None);
        assert_eq!(error_text(9999), None);
        assert!(alarm_text(3).unwrap().contains("reset while moving"));
        assert_eq!(alarm_text(0), None);
        assert_eq!(error_hint(9999), "(unknown code)");
        assert!(error_hint(22).starts_with('(') && error_hint(22).ends_with(')'));
        assert!(alarm_hint(1).contains("hard limit"));
    }
}
