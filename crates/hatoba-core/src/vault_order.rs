//! The custom order of the hosts page (HOST-16).
//!
//! Each host and Telnet host has a place, its `sort`: the order goes by place, and the hosts
//! without one (0) come after the others. Places leave room between them, so moving some hosts
//! writes only those and adding a host writes only it; where two neighbours leave no room, every
//! host is numbered again.

use crate::error::{Error, Result};
use crate::model::Item;
use crate::vault::Vault;

/// The room between neighbouring places when hosts are numbered.
pub const PLACE_STEP: i64 = 1 << 16;

/// The places that put items in the order given, from their places now (0 for none): each
/// greater than the one before. The places of a longest run that already rises stay as they are,
/// and the others are spread out between them.
#[must_use]
pub fn arrange(places: &[i64]) -> Vec<i64> {
    let keep = rising(places);
    let mut out = places.to_vec();
    let mut before = 0i128;
    let mut i = 0;
    while i < places.len() {
        if keep[i] {
            before = i128::from(places[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < places.len() && !keep[i] {
            i += 1;
        }
        let count = (i - start) as i128;
        let after = places
            .get(i)
            .map_or(before + i128::from(PLACE_STEP) * (count + 1), |&p| {
                i128::from(p)
            });
        if after - before <= count {
            return numbered(places.len());
        }
        for (k, slot) in out[start..i].iter_mut().enumerate() {
            let place = before + (after - before) * (k as i128 + 1) / (count + 1);
            match i64::try_from(place) {
                Ok(place) => *slot = place,
                Err(_) => return numbered(places.len()),
            }
        }
    }
    out
}

/// `n` places, [`PLACE_STEP`] apart.
fn numbered(n: usize) -> Vec<i64> {
    (1..=n as i64).map(|i| i * PLACE_STEP).collect()
}

/// Which of `places` make up a longest run of places that rises, leaving out the hosts without one.
fn rising(places: &[i64]) -> Vec<bool> {
    // `tails[n]`: the item that ends the run of length n + 1 with the lowest last place so far.
    let mut tails: Vec<usize> = Vec::new();
    let mut prev: Vec<Option<usize>> = vec![None; places.len()];
    for (i, &place) in places.iter().enumerate() {
        if place <= 0 {
            continue;
        }
        let at = tails.partition_point(|&t| places[t] < place);
        prev[i] = at.checked_sub(1).map(|a| tails[a]);
        if at == tails.len() {
            tails.push(i);
        } else {
            tails[at] = i;
        }
    }
    let mut keep = vec![false; places.len()];
    let mut next = tails.last().copied();
    while let Some(i) = next {
        keep[i] = true;
        next = prev[i];
    }
    keep
}

fn place_of(item: &Item) -> Option<i64> {
    match item {
        Item::Host(host) => Some(host.sort),
        Item::TelnetHost(host) => Some(host.sort),
        _ => None,
    }
}

fn set_place(item: &mut Item, place: i64) {
    match item {
        Item::Host(host) => host.sort = place,
        Item::TelnetHost(host) => host.sort = place,
        _ => {}
    }
}

impl Vault {
    /// Puts the hosts and Telnet hosts in `order` in that order (HOST-16), writing as few of them
    /// as it can, in one transaction. Ids that name neither are skipped, and the hosts `order`
    /// leaves out keep their places.
    ///
    /// # Errors
    /// [`Error::Locked`]; storage errors, after which nothing has changed.
    pub fn arrange_hosts(&mut self, order: &[String]) -> Result<()> {
        if !self.is_unlocked() {
            return Err(Error::Locked);
        }
        let mut ids: Vec<String> = Vec::new();
        let mut places = Vec::new();
        for id in order {
            if let Some(place) = self.get(id).and_then(place_of)
                && !ids.contains(id)
            {
                ids.push(id.clone());
                places.push(place);
            }
        }
        self.write_places(&ids, &places, &arrange(&places))
    }

    /// The place of a host being added, right after the host `after` or else at the end of the
    /// custom order (HOST-16); other hosts move along where there is no room. 0 while no host has
    /// a place, as the custom order has not been made yet, and after a host without one, so the
    /// copy stays beside it among the hosts ordered by name.
    ///
    /// # Errors
    /// [`Error::Locked`]; storage errors.
    pub fn place_new_host(&mut self, after: Option<&str>) -> Result<i64> {
        if !self.is_unlocked() {
            return Err(Error::Locked);
        }
        if let Some(after) = after
            && self.get(after).and_then(place_of) == Some(0)
        {
            return Ok(0);
        }
        let mut placed: Vec<(String, i64)> = self
            .items()
            .filter_map(|(id, item)| place_of(item).map(|place| (id.to_owned(), place)))
            .filter(|(_, place)| *place > 0)
            .collect();
        if placed.is_empty() {
            return Ok(0);
        }
        placed.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        let at = after
            .and_then(|after| placed.iter().position(|(id, _)| id == after))
            .map_or(placed.len(), |i| i + 1);
        placed.insert(at, (String::new(), 0));
        let (ids, places): (Vec<String>, Vec<i64>) = placed.into_iter().unzip();
        let arranged = arrange(&places);
        self.write_places(&ids, &places, &arranged)?;
        Ok(arranged[at])
    }

    /// Writes the places in `arranged` that differ from `places`, skipping the empty id that
    /// stands for a host not saved yet.
    fn write_places(&mut self, ids: &[String], places: &[i64], arranged: &[i64]) -> Result<()> {
        let mut puts = Vec::new();
        for ((id, &old), &new) in ids.iter().zip(places).zip(arranged) {
            if old == new || id.is_empty() {
                continue;
            }
            if let Some(mut item) = self.get(id).cloned() {
                set_place(&mut item, new);
                puts.push((id.clone(), item));
            }
        }
        if puts.is_empty() {
            return Ok(());
        }
        self.commit_cascade(puts, &[])
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::crypto::KdfParams;
    use crate::model::{Host, TelnetHost};
    use crate::vault::ManualClock;

    const STEP: i64 = PLACE_STEP;

    #[test]
    fn a_rising_order_stays_as_it_is() {
        assert_eq!(arrange(&[1, 5, 9]), vec![1, 5, 9]);
        assert_eq!(arrange(&[]), Vec::<i64>::new());
    }

    #[test]
    fn hosts_without_places_are_numbered() {
        assert_eq!(arrange(&[0, 0, 0]), vec![STEP, 2 * STEP, 3 * STEP]);
    }

    #[test]
    fn a_moved_host_alone_gets_a_new_place() {
        // The third host moved to the front.
        assert_eq!(
            arrange(&[3 * STEP, STEP, 2 * STEP, 4 * STEP]),
            vec![STEP / 2, STEP, 2 * STEP, 4 * STEP]
        );
        // The second host moved to the end, which is also the third and fourth moving up: one
        // host changes, not two.
        assert_eq!(
            arrange(&[STEP, 3 * STEP, 4 * STEP, 2 * STEP]),
            vec![STEP, 3 * STEP, 4 * STEP, 5 * STEP]
        );
    }

    #[test]
    fn hosts_moved_together_are_spread_between_their_neighbours() {
        assert_eq!(
            arrange(&[STEP, 0, 0, 4 * STEP]),
            vec![STEP, 2 * STEP, 3 * STEP, 4 * STEP]
        );
    }

    #[test]
    fn hosts_without_places_go_after_the_last_place() {
        assert_eq!(arrange(&[7, 0, 0]), vec![7, 7 + STEP, 7 + 2 * STEP]);
    }

    #[test]
    fn neighbours_without_room_number_every_host_again() {
        assert_eq!(arrange(&[1, 3, 2]), vec![STEP, 2 * STEP, 3 * STEP]);
        assert_eq!(arrange(&[0, 1]), vec![STEP, 2 * STEP]);
        assert_eq!(arrange(&[i64::MAX, 0]), vec![STEP, 2 * STEP]);
    }

    fn vault() -> (Vault, Arc<ManualClock>) {
        let clock = Arc::new(ManualClock::new(1_000_000));
        let mut vault = Vault::open_in_memory().unwrap().with_clock(clock.clone());
        vault
            .create_with_params("correct horse battery staple", KdfParams::for_tests())
            .unwrap();
        (vault, clock)
    }

    fn add(vault: &mut Vault, name: &str, sort: i64) -> String {
        let host = Host {
            name: name.into(),
            address: format!("{name}.example.org"),
            sort,
            ..Host::default()
        };
        vault.put(None, Item::Host(host)).unwrap()
    }

    fn place(vault: &Vault, id: &str) -> i64 {
        vault.get(id).and_then(place_of).unwrap()
    }

    #[test]
    fn arranging_writes_only_the_hosts_that_move() {
        let (mut vault, clock) = vault();
        let a = add(&mut vault, "a", STEP);
        let b = add(&mut vault, "b", 2 * STEP);
        let c = add(&mut vault, "c", 3 * STEP);
        let t = vault
            .put(
                None,
                Item::TelnetHost(TelnetHost {
                    name: "switch".into(),
                    sort: 4 * STEP,
                    ..TelnetHost::default()
                }),
            )
            .unwrap();
        let written = |v: &Vault, id: &str| v.get(id).unwrap().updated_at();
        let before = written(&vault, &a);
        clock.advance(1_000);

        vault
            .arrange_hosts(&[t.clone(), a.clone(), "missing".into(), b.clone(), c.clone()])
            .unwrap();

        assert_eq!(place(&vault, &t), STEP / 2);
        assert_eq!(place(&vault, &a), STEP);
        assert_eq!(written(&vault, &a), before);
        assert!(written(&vault, &t) > before);
    }

    #[test]
    fn a_locked_vault_has_no_order_to_change() {
        let (mut vault, _) = vault();
        vault.lock();
        assert!(matches!(vault.arrange_hosts(&[]), Err(Error::Locked)));
        assert!(matches!(vault.place_new_host(None), Err(Error::Locked)));
    }

    #[test]
    fn a_new_host_has_no_place_until_the_order_is_made() {
        let (mut vault, _) = vault();
        add(&mut vault, "a", 0);
        assert_eq!(vault.place_new_host(None).unwrap(), 0);
    }

    #[test]
    fn a_new_host_goes_last_or_after_the_one_it_copies() {
        let (mut vault, _) = vault();
        let a = add(&mut vault, "a", STEP);
        let b = add(&mut vault, "b", STEP + 1);
        let legacy = add(&mut vault, "legacy", 0);
        assert_eq!(vault.place_new_host(None).unwrap(), STEP + 1 + STEP);
        assert_eq!(vault.place_new_host(Some(&legacy)).unwrap(), 0);
        // No room after `a`: every placed host is numbered again, `b` after the copy.
        assert_eq!(vault.place_new_host(Some(&a)).unwrap(), 2 * STEP);
        assert_eq!(place(&vault, &a), STEP);
        assert_eq!(place(&vault, &b), 3 * STEP);
    }
}
