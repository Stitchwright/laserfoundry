/// GRBL machine states from a `?` status report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineState {
    Idle,
    Run,
    Hold,
    Jog,
    Alarm,
    Door,
    Check,
    Home,
    Sleep,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub state: MachineState,
    /// Machine position in mm, when reported (`MPos`, or `WPos` as a fallback).
    pub position: Option<(f64, f64, f64)>,
    /// The position is in work coordinates (`WPos`), not machine coordinates: it shifts with the
    /// work offset, so it cannot be mapped onto the bed.
    pub position_is_work: bool,
}

/// Parses `<Idle|MPos:1.000,2.000,0.000|FS:0,0>`, or the older comma-separated form Smoothieware (and GRBL 0.9) sends,
/// `<Idle,MPos:1.000,2.000,0.000,WPos:1.000,2.000,0.000>`. Returns `None` for anything else.
pub fn parse_status(line: &str) -> Option<Status> {
    let inner = line.trim().strip_prefix('<')?.strip_suffix('>')?;
    if !inner.contains('|') && inner.contains(',') {
        return parse_legacy_status(inner);
    }
    let mut fields = inner.split('|');
    let state = state_named(fields.next()?.split(':').next()?);
    let mut position_is_work = false;
    let position = fields
        .find_map(|f| {
            f.strip_prefix("MPos:").or_else(|| {
                let w = f.strip_prefix("WPos:");
                position_is_work = w.is_some();
                w
            })
        })
        .and_then(|v| {
            let mut n = v.split(',').map(|s| s.trim().parse::<f64>().ok().filter(|x| x.is_finite()));
            Some((n.next()??, n.next()??, n.next().flatten().unwrap_or(0.0)))
        });
    Some(Status { state, position, position_is_work })
}

fn state_named(name: &str) -> MachineState {
    match name {
        "Idle" => MachineState::Idle,
        "Run" => MachineState::Run,
        "Hold" => MachineState::Hold,
        "Jog" => MachineState::Jog,
        "Alarm" => MachineState::Alarm,
        "Door" => MachineState::Door,
        "Check" => MachineState::Check,
        "Home" => MachineState::Home,
        "Sleep" => MachineState::Sleep,
        _ => MachineState::Unknown,
    }
}

/// The comma-separated form: the state, then `MPos:x,y,z` and/or `WPos:x,y,z` with the coordinates split by commas too.
fn parse_legacy_status(inner: &str) -> Option<Status> {
    let tokens: Vec<&str> = inner.split(',').collect();
    let state = state_named(tokens.first()?.split(':').next()?);
    let coordinates = |prefix: &str| {
        let at = tokens.iter().position(|t| t.starts_with(prefix))?;
        let mut n = tokens.get(at..at + 3)?.iter().enumerate().map(|(i, t)| {
            let t = if i == 0 { t.strip_prefix(prefix)? } else { t };
            t.trim().parse::<f64>().ok().filter(|x| x.is_finite())
        });
        Some((n.next()??, n.next()??, n.next()??))
    };
    let (position, position_is_work) = match (coordinates("MPos:"), coordinates("WPos:")) {
        (Some(m), _) => (Some(m), false),
        (None, Some(w)) => (Some(w), true),
        (None, None) => (None, false),
    };
    Some(Status { state, position, position_is_work })
}

/// Parses the position Marlin answers `M114` with, `X:10.00 Y:20.00 Z:0.00 E:0.00 Count X:800 Y:1600 Z:0`: the first three axes
/// before `Count`. Marlin reports the position in the work coordinates in force, so callers should treat it as one.
pub fn parse_marlin_position(line: &str) -> Option<(f64, f64, f64)> {
    let head = line.split("Count").next()?;
    let axis = |name: &str| {
        let word = head.split_whitespace().find_map(|w| w.strip_prefix(name))?;
        word.parse::<f64>().ok().filter(|v| v.is_finite())
    };
    Some((axis("X:")?, axis("Y:")?, axis("Z:").unwrap_or(0.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_positions_are_flagged_because_they_cannot_be_placed_on_the_bed() {
        let w = parse_status("<Idle|WPos:1.000,2.000,0.000|FS:0,0>").unwrap();
        assert!(w.position_is_work && w.position == Some((1.0, 2.0, 0.0)));
        let m = parse_status("<Idle|MPos:1.000,2.000,0.000|WPos:9,9,9>").unwrap();
        assert!(!m.position_is_work && m.position == Some((1.0, 2.0, 0.0)), "machine position wins when both are reported");
        assert!(!parse_status("<Idle>").unwrap().position_is_work);
    }

    #[test]
    fn parses_state_and_position() {
        let s = parse_status("<Run|MPos:10.500,-2.000,0.000|FS:1000,500>").unwrap();
        assert_eq!(s.state, MachineState::Run);
        assert_eq!(s.position, Some((10.5, -2.0, 0.0)));
        assert_eq!(parse_status("<Hold:0|WPos:1,2,3>").unwrap().state, MachineState::Hold);
    }

    #[test]
    fn the_older_comma_form_is_read_too() {
        let s = parse_status("<Run,MPos:10.500,-2.000,0.000,WPos:1.000,2.000,0.000>").unwrap();
        assert_eq!((s.state, s.position, s.position_is_work), (MachineState::Run, Some((10.5, -2.0, 0.0)), false), "machine position wins");
        let w = parse_status("<Idle,WPos:1.000,2.000,3.000>").unwrap();
        assert_eq!((w.position, w.position_is_work), (Some((1.0, 2.0, 3.0)), true));
        assert_eq!(parse_status("<Hold:0,MPos:1,2,3>").unwrap().state, MachineState::Hold);
        assert_eq!(parse_status("<Idle,Buf:0,RX:0>").unwrap().position, None, "no position reported");
        for bad in ["<Idle,MPos:a,b,c>", "<Idle,MPos:1,2>", "<Idle,MPos:NaN,1,2>", "<,>", "<Idle,MPos:1,2,inf>"] {
            assert!(parse_status(bad).is_none_or(|s| s.position.is_none()), "{bad}");
        }
    }

    #[test]
    fn marlins_position_report_is_read_up_to_the_step_counts() {
        assert_eq!(parse_marlin_position("X:10.00 Y:20.50 Z:0.00 E:0.00 Count X:800 Y:1640 Z:0"), Some((10.0, 20.5, 0.0)));
        assert_eq!(parse_marlin_position("X:-5.25 Y:0.00 Count X:1 Y:2"), Some((-5.25, 0.0, 0.0)), "no Z is fine");
        assert_eq!(parse_marlin_position("X:1 Y:2 Z:3 E:0 Count X:99 Y:99 Z:99"), Some((1.0, 2.0, 3.0)), "the step counts are not the position");
        for junk in ["ok", "", "X:1", "Y:2", "X:a Y:b", "X:NaN Y:1", "X:1 Y:inf", "echo:busy: processing", "Count X:1 Y:2"] {
            assert_eq!(parse_marlin_position(junk), None, "{junk:?}");
        }
    }

    #[test]
    fn rejects_non_status_lines() {
        assert_eq!(parse_status("ok"), None);
        assert_eq!(parse_status("<Idle"), None);
        assert_eq!(parse_status(""), None);
    }

    #[test]
    fn bad_numbers_leave_position_empty() {
        assert_eq!(parse_status("<Idle|MPos:a,b,c>").unwrap().position, None);
        assert_eq!(parse_status("<Idle|MPos:NaN,1,2>").unwrap().position, None);
    }
}
