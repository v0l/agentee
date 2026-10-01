use std::collections::VecDeque;

#[derive(Clone, Copy)]
struct Edge {
    to: usize,
    cap: i32,
    cost: i64,
    rev: usize,
    orig: bool,
}

pub struct Flow {
    adj: Vec<Vec<Edge>>,
}

impl Flow {
    pub fn new(n: usize) -> Flow {
        Flow { adj: vec![Vec::new(); n] }
    }

    pub fn add(&mut self, a: usize, b: usize, cap: i32) {
        self.add_cost(a, b, cap, 0);
    }

    pub fn add_cost(&mut self, a: usize, b: usize, cap: i32, cost: i64) {
        let ra = self.adj[b].len();
        let rb = self.adj[a].len();
        self.adj[a].push(Edge { to: b, cap, cost, rev: ra, orig: true });
        self.adj[b].push(Edge { to: a, cap: 0, cost: -cost, rev: rb, orig: false });
    }

    pub fn min_cost_flow(&mut self, s: usize, t: usize) -> (i32, i64) {
        let n = self.adj.len();
        let (mut flow, mut cost) = (0, 0);
        loop {
            let mut dist = vec![i64::MAX; n];
            let mut prev: Vec<Option<(usize, usize)>> = vec![None; n];
            let mut inq = vec![false; n];
            let mut q = VecDeque::new();
            dist[s] = 0;
            q.push_back(s);
            inq[s] = true;
            while let Some(u) = q.pop_front() {
                inq[u] = false;
                for (k, e) in self.adj[u].iter().enumerate() {
                    if e.cap > 0 && dist[u] != i64::MAX && dist[u] + e.cost < dist[e.to] {
                        dist[e.to] = dist[u] + e.cost;
                        prev[e.to] = Some((u, k));
                        if !inq[e.to] {
                            inq[e.to] = true;
                            q.push_back(e.to);
                        }
                    }
                }
            }
            if dist[t] == i64::MAX {
                return (flow, cost);
            }
            let mut v = t;
            while v != s {
                let (u, k) = prev[v].unwrap();
                let rev = self.adj[u][k].rev;
                self.adj[u][k].cap -= 1;
                self.adj[v][rev].cap += 1;
                v = u;
            }
            flow += 1;
            cost += dist[t];
        }
    }

    pub fn max_flow(&mut self, s: usize, t: usize) -> i32 {
        let mut total = 0;
        loop {
            let mut prev: Vec<Option<(usize, usize)>> = vec![None; self.adj.len()];
            let mut q = VecDeque::new();
            q.push_back(s);
            let mut seen = vec![false; self.adj.len()];
            seen[s] = true;
            while let Some(u) = q.pop_front() {
                if u == t {
                    break;
                }
                for (k, e) in self.adj[u].iter().enumerate() {
                    if e.cap > 0 && !seen[e.to] {
                        seen[e.to] = true;
                        prev[e.to] = Some((u, k));
                        q.push_back(e.to);
                    }
                }
            }
            if !seen[t] {
                return total;
            }
            let mut v = t;
            while v != s {
                let (u, k) = prev[v].unwrap();
                let rev = self.adj[u][k].rev;
                self.adj[u][k].cap -= 1;
                self.adj[v][rev].cap += 1;
                v = u;
            }
            total += 1;
        }
    }

    pub fn take_path(&mut self, s: usize, t: usize) -> Option<Vec<usize>> {
        let mut path = vec![s];
        let mut u = s;
        let mut guard = 0;
        while u != t {
            guard += 1;
            if guard > self.adj.len() {
                return None;
            }
            let k = self.adj[u].iter().position(|e| {
                let back = &self.adj[e.to][e.rev];
                e.orig && back.cap > 0 && !path.contains(&e.to)
            })?;
            let to = self.adj[u][k].to;
            let rev = self.adj[u][k].rev;
            self.adj[u][k].cap += 1;
            self.adj[to][rev].cap -= 1;
            path.push(to);
            u = to;
        }
        Some(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_cost_prefers_the_cheap_exit_and_still_fills() {
        let mut f = Flow::new(5);
        let (s, a, cheap, dear, t) = (0, 1, 2, 3, 4);
        f.add(s, a, 2);
        f.add_cost(a, cheap, 1, 1);
        f.add_cost(a, dear, 1, 10);
        f.add(cheap, t, 1);
        f.add(dear, t, 1);
        assert_eq!(f.min_cost_flow(s, t), (2, 11));
    }

    #[test]
    fn two_sources_share_a_channel_of_capacity_one_only_once() {
        let mut f = Flow::new(6);
        let (s, a, b, c, d, t) = (0, 1, 2, 3, 4, 5);
        f.add(s, a, 1);
        f.add(s, b, 1);
        f.add(a, c, 1);
        f.add(b, c, 1);
        f.add(c, d, 1);
        f.add(d, t, 5);
        f.add(b, d, 1);
        assert_eq!(f.max_flow(s, t), 2);
        let p1 = f.take_path(s, t).unwrap();
        let p2 = f.take_path(s, t).unwrap();
        assert!(p1.contains(&c) != p2.contains(&c));
        assert!(f.take_path(s, t).is_none());
    }
}
