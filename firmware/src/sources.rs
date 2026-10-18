//! Pin at most three official archives and coalesce shared service dates.
use gtfs::{Date, poznan::ArchiveName};

pub struct Archives {
    dates: [Date; 3],
    required: u8,
    each: [Option<ArchiveName>; 3],
}
impl Archives {
    pub fn new(dates: [Date; 3], required: u8) -> Self {
        Self {
            dates,
            required,
            each: [None; 3],
        }
    }
    pub fn consider(&mut self, name: ArchiveName) {
        for i in 0..3 {
            if self.required & (1 << i) != 0
                && name.covers(self.dates[i])
                && self.each[i].is_none_or(|old| name.as_str() > old.as_str())
            {
                self.each[i] = Some(name);
            }
        }
    }
    pub fn sources(&self) -> Option<[(Option<ArchiveName>, u8); 3]> {
        let mut output = [(None, 0); 3];
        for i in 0..3 {
            if self.required & (1 << i) == 0 {
                continue;
            }
            let name = self.each[i]?;
            let slot = output
                .iter()
                .position(|(old, _)| *old == Some(name))
                .unwrap_or_else(|| output.iter().position(|(old, _)| old.is_none()).unwrap());
            output[slot] = (Some(name), output[slot].1 | (1 << i));
        }
        Some(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn listing(mask: u8) -> Archives {
        Archives::new(
            [
                "20261017".parse().unwrap(),
                "20261018".parse().unwrap(),
                "20261019".parse().unwrap(),
            ],
            mask,
        )
    }
    #[test]
    fn midnight_boundary_coalesces_dates_and_requires_tomorrow() {
        let mut plan = listing(7);
        let first = "20261015_20261018.zip".parse().unwrap();
        let next = "20261019_20261031.zip".parse().unwrap();
        plan.consider(first);
        assert!(plan.sources().is_none());
        plan.consider(next);
        assert_eq!(
            plan.sources(),
            Some([(Some(first), 3), (Some(next), 4), (None, 0)])
        );
        let mut today = listing(3);
        today.consider(first);
        assert_eq!(
            today.sources(),
            Some([(Some(first), 3), (None, 0), (None, 0)])
        );
    }
    #[test]
    fn newer_overlapping_updates_take_precedence_and_shared_dates_coalesce() {
        let mut plan = listing(7);
        let common = "20261015_20261031.zip".parse().unwrap();
        plan.consider(common);
        let update = "20261018_20261018.zip".parse().unwrap();
        plan.consider(update);
        assert_eq!(
            plan.sources(),
            Some([(Some(common), 5), (Some(update), 2), (None, 0)])
        );
    }
}
