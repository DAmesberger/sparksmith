//! The operator's batch ledger. A paid request reserves its maximum charge
//! on disk before it goes out, including when a process later stops mid-call.
use crate::Result;
use serde::{Deserialize, Serialize};
use sparksmith_assist::usage::Usage;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Rates {
    pub input_nusd: u64,
    pub cache_write_nusd: u64,
    pub cache_read_nusd: u64,
    pub output_nusd: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    pub provider: String,
    pub model: String,
    pub limit_cents: u64,
    pub rates: Rates,
    pub rate_source: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum Entry {
    Open {
        budget: Budget,
    },
    Reserve {
        run: String,
        request: u32,
        nusd: u64,
    },
    Settle {
        run: String,
        request: u32,
        nusd: u64,
        usage: Usage,
    },
}

impl Rates {
    fn cost(&self, usage: Usage) -> Result<u64> {
        let mut total = 0u64;
        for (count, rate) in [
            (usage.input_tokens, self.input_nusd),
            (usage.cache_write_tokens, self.cache_write_nusd),
            (usage.cache_read_tokens, self.cache_read_nusd),
            (usage.output_tokens, self.output_nusd),
        ] {
            total = total
                .checked_add(
                    count
                        .checked_mul(rate)
                        .ok_or("charge exceeds the ledger range")?,
                )
                .ok_or("charge exceeds the ledger range")?;
        }
        Ok(total)
    }
    fn upper_bound(&self, context: u64, output: u32) -> Result<u64> {
        let max_input = self
            .input_nusd
            .max(self.cache_write_nusd)
            .max(self.cache_read_nusd);
        let input = context
            .checked_mul(max_input)
            .ok_or("input reservation overflows")?;
        input
            .checked_add(
                u64::from(output)
                    .checked_mul(self.output_nusd)
                    .ok_or("output reservation overflows")?,
            )
            .ok_or_else(|| "reservation overflows".into())
    }
}

#[derive(Default, Debug, Serialize)]
pub struct Balance {
    pub confirmed_nusd: u64,
    pub held_nusd: u64,
    pub attempts: u32,
    pub confirmed_requests: u32,
    pub confirmed_usage: Usage,
}

#[derive(Clone, Copy)]
struct Charge {
    reserved: u64,
    receipt: Option<(u64, Usage)>,
}
impl Charge {
    fn balance(self) -> u64 {
        self.receipt.map_or(self.reserved, |(cost, _)| cost)
    }
    fn confirmed_within_bound(self) -> bool {
        self.receipt.is_some_and(|(cost, _)| cost <= self.reserved)
    }
}
type Charges = BTreeMap<(String, u32), Charge>;

pub struct Ledger<'a> {
    path: &'a Path,
    budget: &'a Budget,
}

impl<'a> Ledger<'a> {
    pub fn new(path: &'a Path, budget: &'a Budget) -> Self {
        Self { path, budget }
    }

    fn access<T>(&self, work: impl FnOnce(&mut File, Charges) -> Result<T>) -> Result<T> {
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(self.path)?;
        file.lock()?;
        if file.metadata()?.len() > 0 {
            file.seek(SeekFrom::End(-1))?;
            let mut last = [0];
            file.read_exact(&mut last)?;
            if last[0] != b'\n' {
                return Err("the batch ledger ends with a torn record".into());
            }
            file.rewind()?;
        }
        let mut rows = BufReader::new(&file).lines();
        match rows.next().transpose()? {
            None => {
                append(
                    &mut file,
                    &Entry::Open {
                        budget: self.budget.clone(),
                    },
                )?;
                return work(&mut file, BTreeMap::new());
            }
            Some(line) => {
                if !matches!(serde_json::from_str(&line)?,Entry::Open{budget} if &budget==self.budget)
                {
                    return Err(
                        "the existing batch has a different provider, model, rate card or limit"
                            .into(),
                    );
                }
            }
        }
        let mut amounts = BTreeMap::new();
        for row in rows {
            match serde_json::from_str(&row?)? {
                Entry::Reserve { run, request, nusd } => {
                    if amounts
                        .insert(
                            (run, request),
                            Charge {
                                reserved: nusd,
                                receipt: None,
                            },
                        )
                        .is_some()
                    {
                        return Err("duplicate reservation in the ledger".into());
                    }
                }
                Entry::Settle {
                    run,
                    request,
                    nusd,
                    usage,
                } => {
                    let charge = amounts
                        .get_mut(&(run, request))
                        .ok_or("missing reservation in ledger")?;
                    if charge.receipt.is_some() || self.budget.rates.cost(usage)? != nusd {
                        return Err("invalid or duplicate receipt in ledger".into());
                    }
                    charge.receipt = Some((nusd, usage));
                }
                Entry::Open { .. } => return Err("duplicate batch header".into()),
            }
        }
        work(&mut file, amounts)
    }

    pub fn can_start(&self, context: u64, output: u32) -> Result<bool> {
        let upper = self.budget.rates.upper_bound(context, output)?;
        let cap = self
            .budget
            .limit_cents
            .checked_mul(10_000_000)
            .ok_or("batch limit overflows")?;
        self.access(|_, amounts| {
            let next = amounts
                .values()
                .try_fold(upper, |sum, charge| sum.checked_add(charge.balance()))
                .ok_or("batch balance overflows")?;
            Ok(next < cap
                && amounts
                    .values()
                    .all(|charge| charge.confirmed_within_bound()))
        })
    }
    pub fn reserve(&self, run: &str, request: u32, context: u64, output: u32) -> Result<()> {
        if context == 0
            || output == 0
            || self.budget.rates.cache_read_nusd == 0
            || self.budget.rates.cache_write_nusd == 0
            || self.budget.rate_source.trim().is_empty()
            || self.budget.rates.output_nusd == 0
            || self.budget.rates.input_nusd == 0
        {
            return Err("a paid run needs its model context, output allowance and sourced input and output prices".into());
        }
        let reserve = self.budget.rates.upper_bound(context, output)?;
        self.access(|file, amounts| {
            if amounts
                .values()
                .any(|charge| !charge.confirmed_within_bound())
            {
                return Err(
                    "a prior request has unconfirmed usage or exceeded its reservation".into(),
                );
            }
            if amounts.contains_key(&(run.to_owned(), request)) {
                return Err("run/request already has a reservation".into());
            }
            let total = amounts
                .values()
                .try_fold(reserve, |sum, charge| sum.checked_add(charge.balance()))
                .ok_or("batch balance overflows")?;
            let cap = self
                .budget
                .limit_cents
                .checked_mul(10_000_000)
                .ok_or("batch limit overflows")?;
            if total >= cap {
                return Err("this request would reach the operator's batch limit".into());
            }
            append(
                file,
                &Entry::Reserve {
                    run: run.into(),
                    request,
                    nusd: reserve,
                },
            )
        })
    }
    pub fn settle(&self, run: &str, request: u32, usage: Usage) -> Result<()> {
        let actual = self.budget.rates.cost(usage)?;
        self.access(|file, amounts| {
            let charge = amounts
                .get(&(run.into(), request))
                .ok_or("missing request reservation")?;
            if charge.receipt.is_some() {
                return Err("request already settled".into());
            }
            append(
                file,
                &Entry::Settle {
                    run: run.into(),
                    request,
                    nusd: actual,
                    usage,
                },
            )?;
            if actual > charge.reserved {
                return Err(
                    "provider usage exceeded the declared model bounds; check the rate card".into(),
                );
            }
            Ok(())
        })
    }
    pub fn balances(&self) -> Result<BTreeMap<String, Balance>> {
        self.access(|_, amounts| {
            let mut balance = BTreeMap::<String, Balance>::new();
            for ((run, _), charge) in amounts {
                let row = balance.entry(run).or_default();
                row.attempts = row
                    .attempts
                    .checked_add(1)
                    .ok_or("attempt count overflow")?;
                let cell = if let Some((_, usage)) = charge.receipt {
                    row.confirmed_usage.add(&usage);
                    row.confirmed_requests += 1;
                    &mut row.confirmed_nusd
                } else {
                    &mut row.held_nusd
                };
                *cell = cell
                    .checked_add(charge.balance())
                    .ok_or("batch balance overflows")?;
            }
            Ok(balance)
        })
    }
}
fn append(file: &mut File, row: &Entry) -> Result<()> {
    let mut bytes = serde_json::to_vec(row)?;
    bytes.push(b'\n');
    file.write_all(&bytes)?;
    file.sync_data()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unfinished_requests_count_against_all_runs_and_the_exact_limit_is_refused() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("ledger.jsonl");
        let spec = Budget {
            provider: "test".into(),
            model: "fixture".into(),
            limit_cents: 1,
            rate_source: "fixture".into(),
            rates: Rates {
                input_nusd: 1000,
                cache_write_nusd: 1000,
                cache_read_nusd: 100,
                output_nusd: 1000,
            },
        };
        let ledger = Ledger::new(&path, &spec);
        assert!(!ledger.can_start(9999, 1)?);
        assert!(ledger.reserve("a", 1, 9999, 1).is_err());
        ledger.reserve("a", 1, 4999, 1)?;
        assert!(ledger.reserve("b", 1, 4999, 1).is_err());
        assert_eq!(ledger.balances()?["a"].held_nusd, 5_000_000);
        ledger.settle(
            "a",
            1,
            Usage {
                input_tokens: 100,
                cache_read_tokens: 50,
                output_tokens: 2,
                ..Default::default()
            },
        )?;
        ledger.reserve("b", 1, 4999, 1)?;
        assert!(ledger.reserve("a", 1, 1, 1).is_err());
        assert_eq!(ledger.balances()?["a"].confirmed_nusd, 107_000);
        assert!(
            ledger
                .settle(
                    "b",
                    1,
                    Usage {
                        input_tokens: 5500,
                        ..Default::default()
                    }
                )
                .is_err()
        );
        assert!(!ledger.can_start(1, 1)?);
        assert_eq!(ledger.balances()?["b"].confirmed_nusd, 5_500_000);
        Ok(())
    }
}
