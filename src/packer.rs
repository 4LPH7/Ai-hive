use crate::common::est_tokens;
use std::collections::HashMap;

pub struct Section<'a> {
    pub name: &'a str,
    pub text: &'a str,
    pub weight: f64,
}

pub fn default_section_weight(name: &str) -> f64 {
    match name {
        "memory" => 0.35,
        "playbook" => 0.15,
        "task_notes" => 0.25,
        "code_graph" => 0.25,
        _ => 1.0,
    }
}

pub fn pack(sections: &[Section], budget: usize) -> String {
    if sections.is_empty() || budget == 0 {
        return String::new();
    }

    let active: Vec<&Section> = sections
        .iter()
        .filter(|s| !s.text.trim().is_empty())
        .collect();

    if active.is_empty() {
        return String::new();
    }

    let costs: HashMap<&str, usize> = active
        .iter()
        .map(|s| (s.name, est_tokens(s.text)))
        .collect();

    let w_sum: f64 = active.iter().map(|s| s.weight).sum();
    let safe_w_sum = if w_sum <= 0.0 { 1.0 } else { w_sum };

    let mut alloc: HashMap<&str, usize> = active
        .iter()
        .map(|s| {
            (
                s.name,
                ((budget as f64) * (s.weight / safe_w_sum)).floor() as usize,
            )
        })
        .collect();

    // Dynamic redistribution of surplus
    let mut surplus: usize = alloc
        .iter()
        .map(|(&name, &al)| {
            let cost = costs[name];
            al.saturating_sub(cost)
        })
        .sum();

    for &s in &active {
        let cost = costs[s.name];
        if cost < alloc[s.name] {
            alloc.insert(s.name, cost);
        }
    }

    while surplus > 0 {
        let unsatisfied: Vec<&&Section> = active
            .iter()
            .filter(|s| costs[s.name] > alloc[s.name])
            .collect();

        if unsatisfied.is_empty() {
            break;
        }

        let u_w_sum: f64 = unsatisfied.iter().map(|s| s.weight).sum();
        let safe_u_w_sum = if u_w_sum <= 0.0 { 1.0 } else { u_w_sum };

        let mut distributed = 0;
        for s in &unsatisfied {
            let share = (surplus as f64 * (s.weight / safe_u_w_sum))
                .max(1.0)
                .floor() as usize;
            let needed = costs[s.name] - alloc[s.name];
            let rem = surplus - distributed;
            let add = needed.min(rem.min(share));
            *alloc.get_mut(s.name).unwrap() += add;
            distributed += add;
            if distributed >= surplus {
                break;
            }
        }
        if distributed == 0 {
            break;
        }
        surplus -= distributed;
    }

    let mut out: Vec<String> = Vec::new();
    let mut omitted: Vec<&str> = Vec::new();

    for &s in &active {
        let al = alloc[s.name];
        let c = costs[s.name];
        if al < 15 && c > al {
            omitted.push(s.name);
            continue;
        }
        if c <= al {
            out.push(s.text.trim().to_string());
        } else {
            let suffix = "\n...[truncated to budget]";
            let suffix_tokens = est_tokens(suffix);
            let avail_tokens = if al > suffix_tokens {
                al - suffix_tokens
            } else {
                1
            };
            let lines: Vec<&str> = s.text.trim().lines().collect();
            let mut low = 0usize;
            let mut high = lines.len();
            let mut best = 0usize;

            while low <= high {
                let mid = (low + high) / 2;
                let cand = lines[..mid].join("\n");
                if est_tokens(&cand) <= avail_tokens {
                    best = mid;
                    low = mid + 1;
                } else {
                    if mid == 0 {
                        break;
                    }
                    high = mid - 1;
                }
            }

            if best > 0 {
                let truncated = format!("{}{}", lines[..best].join("\n"), suffix);
                out.push(truncated);
            } else {
                let chars = avail_tokens * 3;
                let mut safe_chars = chars.min(s.text.len());
                while safe_chars > 0 && !s.text.is_char_boundary(safe_chars) {
                    safe_chars -= 1;
                }
                let truncated = format!("{}{}", &s.text[..safe_chars], suffix);
                out.push(truncated);
            }
        }
    }

    let mut result = out.join("\n\n");
    if !omitted.is_empty() {
        result.push_str(&format!("\n\n[omitted_sections: {}]", omitted.join(", ")));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pack_empty() {
        assert_eq!(pack(&[], 1000), "");
    }

    #[test]
    fn test_pack_quota_distribution() {
        let sec1 = Section {
            name: "memory",
            text: "Short memory",
            weight: 0.35,
        };
        let sec2 = Section {
            name: "playbook",
            text: "Short playbook",
            weight: 0.15,
        };
        let res = pack(&[sec1, sec2], 1500);
        assert!(res.contains("Short memory"));
        assert!(res.contains("Short playbook"));
    }

    #[test]
    fn test_pack_truncation_and_omitted() {
        let long_text = (0..500)
            .map(|i| format!("Line {i} with substantial detailed technical text"))
            .collect::<Vec<_>>()
            .join("\n");
        let sec1 = Section {
            name: "memory",
            text: &long_text,
            weight: 0.35,
        };
        let sec2 = Section {
            name: "playbook",
            text: "Playbook rule",
            weight: 0.15,
        };
        let res = pack(&[sec1, sec2], 100);
        assert!(res.contains("...[truncated to budget]"));
    }
}
