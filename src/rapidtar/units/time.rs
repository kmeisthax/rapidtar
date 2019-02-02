use std::fmt;
use std::fmt::{Display, Formatter};
use std::time::Duration;

/// Wrapper structure for printing durations in human printable format.
pub struct HRDuration {
    inner: Duration
}

impl From<Duration> for HRDuration {
    fn from(duration: Duration) -> HRDuration {
        HRDuration {
            inner: duration
        }
    }
}

impl Into<Duration> for HRDuration {
    fn into(self) -> Duration {
        self.inner
    }
}

impl Display for HRDuration {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let total_secs = self.inner.as_secs();
        let remain_nanos = self.inner.subsec_nanos();
        
        let days = total_secs / (60 * 60 * 24);
        let days_remain_secs = total_secs - (days * 60 * 60 * 24);
        let hours = days_remain_secs / (60 * 60);
        let hours_remain_secs = days_remain_secs - (hours * 60 * 60);
        let minutes = hours_remain_secs / (60);
        let seconds = hours_remain_secs - (minutes * 60);
        
        let millis = remain_nanos / (1000 * 1000);
        let millis_remain_nanos = remain_nanos - (millis * 1000 * 1000);
        let micros = millis_remain_nanos / (1000);
        let nanos = millis_remain_nanos - (micros * 1000);
        
        if days > 0 {
            write!(f, "{}d", days)?;
        }
        
        if hours > 0 {
            write!(f, "{}h", hours)?;
        }
        
        if minutes > 0 {
            write!(f, "{}m", minutes)?;
        }
        
        if seconds > 0 {
            write!(f, "{}s", seconds)?;
        }
        
        if millis > 0 {
            write!(f, "{}ms", millis)?;
        }
        
        if micros > 0 {
            write!(f, "{}μs", micros)?;
        }
        
        if nanos > 0 {
            write!(f, "{}ns", nanos)?;
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use rapidtar::units::time::HRDuration;
    use std::time::Duration;
    
    #[test]
    fn time_hours() {
        let my_time = Duration::new(12*60*60 + 30*60 + 14, 0);
        let fmtd = format!("{}", HRDuration::from(my_time));
        
        assert_eq!(fmtd, "12h30m14s");
    }
    
    #[test]
    fn time_days() {
        let my_time = Duration::new(14*24*60*60 + 12*60*60 + 30*60 + 14, 0);
        let fmtd = format!("{}", HRDuration::from(my_time));
        
        assert_eq!(fmtd, "14d12h30m14s");
    }
    
    #[test]
    fn time_nanos() {
        let my_time = Duration::new(30*60 + 14, 123);
        let fmtd = format!("{}", HRDuration::from(my_time));
        
        assert_eq!(fmtd, "30m14s123ns");
    }
}