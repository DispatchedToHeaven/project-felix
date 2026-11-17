//! Russian draughts: the full rules and a small engine for NPCs. No mod API types here.
//!
//! The 8x8 board's dark squares (a1 is dark), white moves first from ranks 1 to 3. Men move one
//! square diagonally forward and capture forward and backward; kings fly any distance. Taking is
//! compulsory and goes on while it can (any sequence, not the longest); taken pieces leave the
//! board when the move ends and can't be jumped twice. A man reaching the last rank while taking
//! becomes a king and goes on taking as one. A side with no moves loses; thirty moves each of
//! kings only with nothing taken, or a position seen three times, is a draw.
//! Squares are `rank * 8 + file`, as in `chess`.

use crate::chess::{Color, square_name};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Man {
    pub color: Color,
    pub king: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Move {
    /// Every square stood on, from the start to the end.
    pub path: Vec<u8>,
    pub taken: Vec<u8>,
}

impl Move {
    pub fn from(&self) -> u8 {
        self.path[0]
    }
    pub fn to(&self) -> u8 {
        *self.path.last().unwrap()
    }
    pub fn text(&self) -> String {
        let sep = if self.taken.is_empty() { "-" } else { ":" };
        self.path.iter().map(|s| square_name(*s)).collect::<Vec<_>>().join(sep)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Win(Color),
    Draw,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Position {
    pub board: [Option<Man>; 64],
    pub side: Color,
    /// Plies in a row of kings only, with nothing taken.
    pub quiet: u32,
}

const DIRS: [(i8, i8); 4] = [(1, 1), (-1, 1), (-1, -1), (1, -1)];

fn step(sq: u8, df: i8, dr: i8) -> Option<u8> {
    let (f, r) = ((sq % 8) as i8 + df, (sq / 8) as i8 + dr);
    ((0..8).contains(&f) && (0..8).contains(&r)).then(|| (r * 8 + f) as u8)
}

pub fn dark(sq: u8) -> bool {
    (sq % 8 + sq / 8) % 2 == 0
}

impl Position {
    pub fn start() -> Position {
        let mut board = [None; 64];
        for sq in 0..64u8 {
            if dark(sq) {
                let r = sq / 8;
                if r <= 2 {
                    board[sq as usize] = Some(Man { color: Color::White, king: false });
                } else if r >= 5 {
                    board[sq as usize] = Some(Man { color: Color::Black, king: false });
                }
            }
        }
        Position { board, side: Color::White, quiet: 0 }
    }

    /// One letter a square: `w`/`b` men, `W`/`B` kings, `.` empty; a1 first, rank by rank.
    pub fn board_text(&self) -> String {
        self.board
            .iter()
            .map(|m| match m {
                None => '.',
                Some(Man { color: Color::White, king: false }) => 'w',
                Some(Man { color: Color::White, king: true }) => 'W',
                Some(Man { color: Color::Black, king: false }) => 'b',
                Some(Man { color: Color::Black, king: true }) => 'B',
            })
            .collect()
    }

    pub fn from_text(board: &str, side: Color) -> Option<Position> {
        let mut b = [None; 64];
        for (i, c) in board.chars().enumerate().take(64) {
            b[i] = match c {
                'w' => Some(Man { color: Color::White, king: false }),
                'W' => Some(Man { color: Color::White, king: true }),
                'b' => Some(Man { color: Color::Black, king: false }),
                'B' => Some(Man { color: Color::Black, king: true }),
                _ => None,
            };
        }
        Some(Position { board: b, side, quiet: 0 })
    }

    fn last_rank(color: Color) -> u8 {
        if color == Color::White { 7 } else { 0 }
    }

    /// Every way to go on taking from `at` with the piece `man` (it left `start`; `taken` are
    /// already jumped). Adds finished sequences to `out`.
    fn takes(&self, start: u8, at: u8, man: Man, path: &mut Vec<u8>, taken: &mut Vec<u8>, out: &mut Vec<Move>) {
        let free = |s: u8| s == start || self.board[s as usize].is_none();
        let enemy = |s: u8| self.board[s as usize].is_some_and(|m| m.color != man.color);
        for &(df, dr) in &DIRS {
            // Find the piece to jump: next to a man, anywhere along the line for a king.
            let mut s = at;
            let mut over = None;
            while let Some(n) = step(s, df, dr) {
                if enemy(n) && !taken.contains(&n) {
                    over = Some(n);
                    break;
                }
                if !free(n) || !man.king {
                    break;
                }
                s = n;
            }
            let Some(over) = over else { continue };
            // Where it can land: right behind for a man, any free square behind for a king.
            let mut landings = Vec::new();
            let mut l = over;
            while let Some(n) = step(l, df, dr) {
                if !free(n) {
                    break;
                }
                landings.push(n);
                if !man.king {
                    break;
                }
                l = n;
            }
            if landings.is_empty() {
                continue;
            }
            // A king must land where it can go on taking, if any landing allows it.
            let mut options = Vec::new();
            for &land in &landings {
                let crowned = man.king || land / 8 == Self::last_rank(man.color);
                let next = Man { color: man.color, king: crowned };
                taken.push(over);
                path.push(land);
                let mut more = Vec::new();
                self.takes(start, land, next, path, taken, &mut more);
                path.pop();
                taken.pop();
                options.push((land, next, more));
            }
            let continuing = options.iter().any(|(_, _, more)| !more.is_empty());
            for (land, _next, more) in options {
                if continuing {
                    out.extend(more);
                } else {
                    let mut p = path.clone();
                    p.push(land);
                    let mut t = taken.clone();
                    t.push(over);
                    out.push(Move { path: p, taken: t });
                }
            }
        }
    }

    pub fn legal_moves(&self) -> Vec<Move> {
        let mut caps = Vec::new();
        for sq in 0..64u8 {
            let Some(m) = self.board[sq as usize] else { continue };
            if m.color != self.side {
                continue;
            }
            self.takes(sq, sq, m, &mut vec![sq], &mut Vec::new(), &mut caps);
        }
        if !caps.is_empty() {
            caps.sort_by(|a, b| a.path.cmp(&b.path));
            caps.dedup();
            return caps;
        }
        let mut out = Vec::new();
        let forward: i8 = if self.side == Color::White { 1 } else { -1 };
        for sq in 0..64u8 {
            let Some(m) = self.board[sq as usize] else { continue };
            if m.color != self.side {
                continue;
            }
            for &(df, dr) in &DIRS {
                if !m.king && dr != forward {
                    continue;
                }
                let mut s = sq;
                while let Some(n) = step(s, df, dr) {
                    if self.board[n as usize].is_some() {
                        break;
                    }
                    out.push(Move { path: vec![sq, n], taken: Vec::new() });
                    if !m.king {
                        break;
                    }
                    s = n;
                }
            }
        }
        out
    }

    pub fn play(&self, m: &Move) -> Position {
        let mut n = self.clone();
        let mut man = n.board[m.from() as usize].take().expect("a piece to move");
        for t in &m.taken {
            n.board[*t as usize] = None;
        }
        let was_king = man.king;
        if m.path.iter().skip(1).any(|s| s / 8 == Self::last_rank(man.color)) {
            man.king = true;
        }
        n.board[m.to() as usize] = Some(man);
        n.quiet = if m.taken.is_empty() && was_king { self.quiet + 1 } else { 0 };
        n.side = self.side.other();
        n
    }

    /// The move from `from` to `to` (the one taking most, if several).
    pub fn find(&self, from: u8, to: u8) -> Option<Move> {
        self.legal_moves().into_iter().filter(|m| m.from() == from && m.to() == to).max_by_key(|m| m.taken.len())
    }

    pub fn outcome(&self, history: &[String]) -> Option<Outcome> {
        if self.legal_moves().is_empty() {
            return Some(Outcome::Win(self.side.other()));
        }
        if self.quiet >= 60 {
            return Some(Outcome::Draw);
        }
        let key = self.key();
        if history.iter().filter(|k| **k == key).count() >= 2 {
            return Some(Outcome::Draw);
        }
        None
    }

    pub fn key(&self) -> String {
        format!("{}{}", self.board_text(), if self.side == Color::White { 'w' } else { 'b' })
    }
}

// ---- the engine ---------------------------------------------------------------------------------

/// The position from the side to move's view.
pub fn evaluate(pos: &Position) -> i32 {
    let mut s = 0;
    for (sq, m) in pos.board.iter().enumerate() {
        let Some(m) = m else { continue };
        let r = (sq / 8) as i32;
        let advance = if m.color == Color::White { r } else { 7 - r };
        let f = (sq % 8) as i32;
        let centre = 3 - (2 * f - 7).abs() / 2;
        let v = if m.king { 300 } else { 100 + advance * 6 } + centre * 2;
        s += if m.color == pos.side { v } else { -v };
    }
    s
}

const WIN: i32 = 100_000;

fn search(pos: &Position, depth: u32, mut alpha: i32, beta: i32, ply: i32) -> i32 {
    let moves = pos.legal_moves();
    if moves.is_empty() {
        return -WIN + ply;
    }
    // Keep going through forced takes, so a depth never ends in the middle of an exchange.
    if depth == 0 && moves[0].taken.is_empty() {
        return evaluate(pos);
    }
    let mut best = -WIN;
    for m in &moves {
        let score = -search(&pos.play(m), depth.saturating_sub(1), -beta, -alpha, ply + 1);
        best = best.max(score);
        alpha = alpha.max(score);
        if alpha >= beta || ply > 40 {
            break;
        }
    }
    best
}

/// The engine's move, as `chess::best_move` chooses.
pub fn best_move(pos: &Position, depth: u32, seed: u64, slack: i32) -> Option<Move> {
    let moves = pos.legal_moves();
    let mut scored = Vec::new();
    let mut alpha = -WIN;
    for m in moves {
        let score = -search(&pos.play(&m), depth.saturating_sub(1), -WIN, -alpha + slack + 1, 1);
        alpha = alpha.max(score);
        scored.push((score, m));
    }
    let best = scored.iter().map(|s| s.0).max()?;
    let good: Vec<Move> = scored.into_iter().filter(|s| s.0 >= best - slack).map(|s| s.1).collect();
    good.get((seed % good.len() as u64) as usize).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chess::parse_square as sq;

    fn pos(pieces: &[(&str, char)], side: Color) -> Position {
        let mut b = ['.'; 64];
        for (s, c) in pieces {
            b[sq(s).unwrap() as usize] = *c;
        }
        Position::from_text(&b.iter().collect::<String>(), side).unwrap()
    }

    #[test]
    fn opening_moves() {
        let p = Position::start();
        assert_eq!(p.board.iter().filter(|m| m.is_some()).count(), 24);
        assert_eq!(p.legal_moves().len(), 7);
    }

    #[test]
    fn taking_is_compulsory_and_backwards() {
        // A white man on d4, black men on e5 (forward) and c3 (behind): both can be taken, nothing else.
        let p = pos(&[("d4", 'w'), ("e5", 'b'), ("c3", 'b'), ("h2", 'w')], Color::White);
        let mut moves: Vec<String> = p.legal_moves().iter().map(|m| m.text()).collect();
        moves.sort();
        assert_eq!(moves, ["d4:b2", "d4:f6"]);
    }

    #[test]
    fn multiple_take_and_crowning_mid_take() {
        // a1 takes b2, d4 and f6 in one move: a1:c3:e5:g7.
        let p = pos(&[("a1", 'w'), ("b2", 'b'), ("d4", 'b'), ("f6", 'b')], Color::White);
        let m = p.legal_moves();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].text(), "a1:c3:e5:g7");
        let after = p.play(&m[0]);
        assert_eq!(after.board.iter().filter(|x| x.is_some()).count(), 1);

        // A man crowned on the last rank while taking goes on taking as a king.
        let p = pos(&[("b6", 'w'), ("c7", 'b'), ("e7", 'b')], Color::White);
        // b6 takes c7 landing d8 (crowned), then as a king takes e7 landing f6/g5/h4.
        let mut moves: Vec<String> = p.legal_moves().iter().map(|m| m.text()).collect();
        moves.sort();
        assert_eq!(moves, ["b6:d8:f6", "b6:d8:g5", "b6:d8:h4"]);
        let after = p.play(&p.legal_moves()[0]);
        assert!(after.board.iter().flatten().any(|m| m.king));
    }

    #[test]
    fn flying_king_and_no_double_jump() {
        let p = pos(&[("a1", 'W'), ("d4", 'b'), ("h8", 'b')], Color::White);
        let mut moves: Vec<String> = p.legal_moves().iter().map(|m| m.text()).collect();
        moves.sort();
        // Over d4, landing e5, f6 or g7 (h8 is taken by a black man and can't be jumped: no square behind).
        assert_eq!(moves, ["a1:e5", "a1:f6", "a1:g7"]);
    }

    #[test]
    fn a_side_without_moves_loses_and_engine_takes() {
        let p = pos(&[("a1", 'w'), ("b2", 'b'), ("c3", 'b')], Color::White);
        assert_eq!(p.outcome(&[]), Some(Outcome::Win(Color::Black)));
        let p = pos(&[("c3", 'w'), ("d4", 'b'), ("h8", 'b')], Color::White);
        assert_eq!(best_move(&p, 4, 0, 0).unwrap().text(), "c3:e5");
    }
}
