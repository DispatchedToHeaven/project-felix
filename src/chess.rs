//! Chess: the full rules and a small engine for NPCs to play with. No mod API types here.
//!
//! Squares are `rank * 8 + file` (a1 = 0, h8 = 63). Moves are written as in UCI (`e2e4`,
//! `e7e8q`). Rules: castling, en passant, promotion (any piece), check, checkmate, stalemate,
//! the fifty-move rule, threefold repetition and insufficient material (king against king,
//! king and one minor piece against king, bishops all on one colour).

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Color {
    White,
    Black,
}

impl Color {
    pub fn other(self) -> Color {
        match self {
            Color::White => Color::Black,
            Color::Black => Color::White,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Pawn,
    Knight,
    Bishop,
    Rook,
    Queen,
    King,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Piece {
    pub color: Color,
    pub kind: Kind,
}

impl Piece {
    /// FEN letter: upper case for white.
    pub fn letter(self) -> char {
        let c = match self.kind {
            Kind::Pawn => 'p',
            Kind::Knight => 'n',
            Kind::Bishop => 'b',
            Kind::Rook => 'r',
            Kind::Queen => 'q',
            Kind::King => 'k',
        };
        if self.color == Color::White { c.to_ascii_uppercase() } else { c }
    }
    pub fn from_letter(c: char) -> Option<Piece> {
        let kind = match c.to_ascii_lowercase() {
            'p' => Kind::Pawn,
            'n' => Kind::Knight,
            'b' => Kind::Bishop,
            'r' => Kind::Rook,
            'q' => Kind::Queen,
            'k' => Kind::King,
            _ => return None,
        };
        Some(Piece { color: if c.is_ascii_uppercase() { Color::White } else { Color::Black }, kind })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Move {
    pub from: u8,
    pub to: u8,
    pub promotion: Option<Kind>,
}

impl Move {
    pub fn uci(self) -> String {
        let mut s = format!("{}{}", square_name(self.from), square_name(self.to));
        if let Some(k) = self.promotion {
            s.push(Piece { color: Color::Black, kind: k }.letter());
        }
        s
    }
}

pub fn square_name(sq: u8) -> String {
    format!("{}{}", (b'a' + sq % 8) as char, sq / 8 + 1)
}

pub fn parse_square(s: &str) -> Option<u8> {
    let b = s.as_bytes();
    if b.len() != 2 || !(b'a'..=b'h').contains(&b[0]) || !(b'1'..=b'8').contains(&b[1]) {
        return None;
    }
    Some((b[1] - b'1') * 8 + (b[0] - b'a'))
}

const WK: u8 = 1;
const WQ: u8 = 2;
const BK: u8 = 4;
const BQ: u8 = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Position {
    pub board: [Option<Piece>; 64],
    pub side: Color,
    /// Castling rights: bits for white king/queen side, black king/queen side.
    pub castling: u8,
    /// The square a pawn may be taken en passant on.
    pub en_passant: Option<u8>,
    pub halfmove: u32,
    pub fullmove: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Checkmate(Color),
    Stalemate,
    FiftyMoves,
    Repetition,
    InsufficientMaterial,
}

pub const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

const KNIGHT: [(i8, i8); 8] = [(1, 2), (2, 1), (2, -1), (1, -2), (-1, -2), (-2, -1), (-2, 1), (-1, 2)];
const KING: [(i8, i8); 8] = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];
const DIAG: [(i8, i8); 4] = [(1, 1), (-1, 1), (-1, -1), (1, -1)];
const ORTHO: [(i8, i8); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

fn step(sq: u8, df: i8, dr: i8) -> Option<u8> {
    let (f, r) = ((sq % 8) as i8 + df, (sq / 8) as i8 + dr);
    ((0..8).contains(&f) && (0..8).contains(&r)).then(|| (r * 8 + f) as u8)
}

impl Position {
    pub fn start() -> Position {
        Position::from_fen(START).unwrap()
    }

    pub fn from_fen(fen: &str) -> Result<Position, String> {
        let parts: Vec<&str> = fen.split_whitespace().collect();
        if parts.len() < 4 {
            return Err("a FEN has at least four fields".into());
        }
        let mut board = [None; 64];
        let ranks: Vec<&str> = parts[0].split('/').collect();
        if ranks.len() != 8 {
            return Err("eight ranks expected".into());
        }
        for (i, row) in ranks.iter().enumerate() {
            let rank = 7 - i;
            let mut file = 0usize;
            for c in row.chars() {
                if let Some(d) = c.to_digit(10) {
                    file += d as usize;
                } else {
                    let p = Piece::from_letter(c).ok_or_else(|| format!("bad piece {c}"))?;
                    if file > 7 {
                        return Err("rank too long".into());
                    }
                    board[rank * 8 + file] = Some(p);
                    file += 1;
                }
            }
            if file != 8 {
                return Err("rank of the wrong length".into());
            }
        }
        let side = if parts[1] == "b" { Color::Black } else { Color::White };
        let mut castling = 0;
        for c in parts[2].chars() {
            castling |= match c {
                'K' => WK,
                'Q' => WQ,
                'k' => BK,
                'q' => BQ,
                _ => 0,
            };
        }
        let en_passant = parse_square(parts[3]);
        let halfmove = parts.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
        let fullmove = parts.get(5).and_then(|s| s.parse().ok()).unwrap_or(1);
        Ok(Position { board, side, castling, en_passant, halfmove, fullmove })
    }

    pub fn fen(&self) -> String {
        let mut s = String::new();
        for rank in (0..8).rev() {
            let mut empty = 0;
            for file in 0..8 {
                match self.board[rank * 8 + file] {
                    Some(p) => {
                        if empty > 0 {
                            s.push_str(&empty.to_string());
                            empty = 0;
                        }
                        s.push(p.letter());
                    }
                    None => empty += 1,
                }
            }
            if empty > 0 {
                s.push_str(&empty.to_string());
            }
            if rank > 0 {
                s.push('/');
            }
        }
        s.push(' ');
        s.push(if self.side == Color::White { 'w' } else { 'b' });
        s.push(' ');
        let mut c = String::new();
        for (bit, ch) in [(WK, 'K'), (WQ, 'Q'), (BK, 'k'), (BQ, 'q')] {
            if self.castling & bit != 0 {
                c.push(ch);
            }
        }
        s.push_str(if c.is_empty() { "-" } else { &c });
        s.push(' ');
        s.push_str(&self.en_passant.map(square_name).unwrap_or_else(|| "-".into()));
        s.push_str(&format!(" {} {}", self.halfmove, self.fullmove));
        s
    }

    /// The board, side, castling and en passant: what repetition compares.
    pub fn key(&self) -> String {
        let f = self.fen();
        f.rsplitn(3, ' ').nth(2).unwrap_or(&f).to_string()
    }

    pub fn king(&self, color: Color) -> Option<u8> {
        (0..64u8).find(|&s| self.board[s as usize] == Some(Piece { color, kind: Kind::King }))
    }

    /// Whether `sq` is attacked by a piece of `by`.
    pub fn attacked(&self, sq: u8, by: Color) -> bool {
        let at = |s: Option<u8>, kinds: &[Kind]| {
            s.and_then(|s| self.board[s as usize]).is_some_and(|p| p.color == by && kinds.contains(&p.kind))
        };
        let pawn_dir: i8 = if by == Color::White { -1 } else { 1 };
        if at(step(sq, 1, pawn_dir), &[Kind::Pawn]) || at(step(sq, -1, pawn_dir), &[Kind::Pawn]) {
            return true;
        }
        if KNIGHT.iter().any(|&(f, r)| at(step(sq, f, r), &[Kind::Knight])) {
            return true;
        }
        if KING.iter().any(|&(f, r)| at(step(sq, f, r), &[Kind::King])) {
            return true;
        }
        for (dirs, kinds) in [(&DIAG, [Kind::Bishop, Kind::Queen]), (&ORTHO, [Kind::Rook, Kind::Queen])] {
            for &(f, r) in dirs.iter() {
                let mut s = sq;
                while let Some(n) = step(s, f, r) {
                    if let Some(p) = self.board[n as usize] {
                        if p.color == by && kinds.contains(&p.kind) {
                            return true;
                        }
                        break;
                    }
                    s = n;
                }
            }
        }
        false
    }

    pub fn in_check(&self, color: Color) -> bool {
        self.king(color).is_some_and(|k| self.attacked(k, color.other()))
    }

    fn pseudo(&self, out: &mut Vec<Move>) {
        let us = self.side;
        for from in 0..64u8 {
            let Some(p) = self.board[from as usize] else { continue };
            if p.color != us {
                continue;
            }
            let mut add = |to: u8| out.push(Move { from, to, promotion: None });
            match p.kind {
                Kind::Pawn => {
                    let dir: i8 = if us == Color::White { 1 } else { -1 };
                    let start_rank = if us == Color::White { 1 } else { 6 };
                    let last_rank = if us == Color::White { 7 } else { 0 };
                    let push = |to: u8, out: &mut Vec<Move>| {
                        if to / 8 == last_rank {
                            for k in [Kind::Queen, Kind::Rook, Kind::Bishop, Kind::Knight] {
                                out.push(Move { from, to, promotion: Some(k) });
                            }
                        } else {
                            out.push(Move { from, to, promotion: None });
                        }
                    };
                    if let Some(one) = step(from, 0, dir).filter(|s| self.board[*s as usize].is_none()) {
                        push(one, out);
                        if from / 8 == start_rank
                            && let Some(two) = step(one, 0, dir).filter(|s| self.board[*s as usize].is_none())
                        {
                            out.push(Move { from, to: two, promotion: None });
                        }
                    }
                    for df in [-1, 1] {
                        if let Some(to) = step(from, df, dir) {
                            let enemy = self.board[to as usize].is_some_and(|q| q.color != us);
                            if enemy || self.en_passant == Some(to) {
                                push(to, out);
                            }
                        }
                    }
                }
                Kind::Knight | Kind::King => {
                    let offsets = if p.kind == Kind::Knight { &KNIGHT } else { &KING };
                    for &(f, r) in offsets.iter() {
                        if let Some(to) = step(from, f, r)
                            && self.board[to as usize].is_none_or(|q| q.color != us)
                        {
                            add(to);
                        }
                    }
                    if p.kind == Kind::King {
                        self.castles(from, out);
                    }
                }
                _ => {
                    let dirs: &[(i8, i8)] = match p.kind {
                        Kind::Bishop => &DIAG,
                        Kind::Rook => &ORTHO,
                        _ => &[(1, 1), (-1, 1), (-1, -1), (1, -1), (1, 0), (0, 1), (-1, 0), (0, -1)],
                    };
                    for &(f, r) in dirs {
                        let mut s = from;
                        while let Some(to) = step(s, f, r) {
                            match self.board[to as usize] {
                                None => add(to),
                                Some(q) => {
                                    if q.color != us {
                                        add(to);
                                    }
                                    break;
                                }
                            }
                            s = to;
                        }
                    }
                }
            }
        }
    }

    fn castles(&self, from: u8, out: &mut Vec<Move>) {
        let us = self.side;
        let (rank, k, q) = if us == Color::White { (0u8, WK, WQ) } else { (56u8, BK, BQ) };
        if from != rank + 4 || self.attacked(from, us.other()) {
            return;
        }
        let empty = |s: u8| self.board[s as usize].is_none();
        let safe = |s: u8| !self.attacked(s, us.other());
        let rook = |s: u8| self.board[s as usize] == Some(Piece { color: us, kind: Kind::Rook });
        if self.castling & k != 0 && empty(rank + 5) && empty(rank + 6) && safe(rank + 5) && safe(rank + 6) && rook(rank + 7) {
            out.push(Move { from, to: rank + 6, promotion: None });
        }
        if self.castling & q != 0
            && empty(rank + 3)
            && empty(rank + 2)
            && empty(rank + 1)
            && safe(rank + 3)
            && safe(rank + 2)
            && rook(rank)
        {
            out.push(Move { from, to: rank + 2, promotion: None });
        }
    }

    /// The position after `m` (which must come from [`Position::legal_moves`]).
    pub fn play(&self, m: Move) -> Position {
        let mut n = self.clone();
        let p = n.board[m.from as usize].take().expect("a piece to move");
        let captured = n.board[m.to as usize];
        n.halfmove += 1;
        if p.kind == Kind::Pawn || captured.is_some() {
            n.halfmove = 0;
        }
        // En passant: the taken pawn is beside the square moved to.
        if p.kind == Kind::Pawn && Some(m.to) == self.en_passant && captured.is_none() {
            let taken = if p.color == Color::White { m.to - 8 } else { m.to + 8 };
            n.board[taken as usize] = None;
        }
        n.en_passant = None;
        if p.kind == Kind::Pawn && (m.to as i8 - m.from as i8).abs() == 16 {
            n.en_passant = Some((m.from + m.to) / 2);
        }
        // Castling moves the rook too.
        if p.kind == Kind::King && (m.to as i8 - m.from as i8).abs() == 2 {
            let (rf, rt) = if m.to > m.from { (m.from + 3, m.from + 1) } else { (m.from - 4, m.from - 1) };
            n.board[rt as usize] = n.board[rf as usize].take();
        }
        n.board[m.to as usize] = Some(match m.promotion {
            Some(kind) => Piece { color: p.color, kind },
            None => p,
        });
        for sq in [m.from, m.to] {
            n.castling &= match sq {
                0 => !WQ,
                7 => !WK,
                4 => !(WK | WQ),
                56 => !BQ,
                63 => !BK,
                60 => !(BK | BQ),
                _ => 0xFF,
            };
        }
        if self.side == Color::Black {
            n.fullmove += 1;
        }
        n.side = self.side.other();
        n
    }

    pub fn legal_moves(&self) -> Vec<Move> {
        let mut pseudo = Vec::with_capacity(48);
        self.pseudo(&mut pseudo);
        pseudo.into_iter().filter(|m| !self.play(*m).in_check(self.side)).collect()
    }

    pub fn parse_move(&self, uci: &str) -> Option<Move> {
        let from = parse_square(uci.get(0..2)?)?;
        let to = parse_square(uci.get(2..4)?)?;
        let promotion = uci.chars().nth(4).and_then(Piece::from_letter).map(|p| p.kind);
        self.legal_moves().into_iter().find(|m| m.from == from && m.to == to && m.promotion == promotion)
    }

    fn insufficient(&self) -> bool {
        let mut minors = Vec::new();
        for (sq, p) in self.board.iter().enumerate() {
            match p.map(|p| p.kind) {
                None | Some(Kind::King) => {}
                Some(Kind::Knight) | Some(Kind::Bishop) => minors.push((sq, p.unwrap().kind)),
                _ => return false,
            }
        }
        match minors.as_slice() {
            [] | [_] => true,
            list => {
                // Bishops only, all on squares of one colour.
                list.iter().all(|(_, k)| *k == Kind::Bishop)
                    && list.iter().map(|(s, _)| (s % 8 + s / 8) % 2).collect::<std::collections::HashSet<_>>().len() == 1
            }
        }
    }

    /// How the game stands after this position, given the keys of every position before it.
    pub fn outcome(&self, history: &[String]) -> Option<Outcome> {
        if self.legal_moves().is_empty() {
            return Some(if self.in_check(self.side) { Outcome::Checkmate(self.side.other()) } else { Outcome::Stalemate });
        }
        if self.halfmove >= 100 {
            return Some(Outcome::FiftyMoves);
        }
        if self.insufficient() {
            return Some(Outcome::InsufficientMaterial);
        }
        let key = self.key();
        if history.iter().filter(|k| **k == key).count() >= 2 {
            return Some(Outcome::Repetition);
        }
        None
    }

    pub fn perft(&self, depth: u32) -> u64 {
        if depth == 0 {
            return 1;
        }
        let moves = self.legal_moves();
        if depth == 1 {
            return moves.len() as u64;
        }
        moves.iter().map(|m| self.play(*m).perft(depth - 1)).sum()
    }
}

// ---- the engine ---------------------------------------------------------------------------------

fn value(k: Kind) -> i32 {
    match k {
        Kind::Pawn => 100,
        Kind::Knight => 320,
        Kind::Bishop => 330,
        Kind::Rook => 500,
        Kind::Queen => 900,
        Kind::King => 0,
    }
}

/// Small bonuses for good squares (white's view; mirrored for black).
fn square_bonus(p: Piece, sq: u8) -> i32 {
    let (f, r) = ((sq % 8) as i32, (sq / 8) as i32);
    let r = if p.color == Color::White { r } else { 7 - r };
    let centre = 3 - ((2 * f - 7).abs() / 2).max((2 * r - 7).abs() / 2);
    match p.kind {
        Kind::Pawn => r * 6 + if (3..=4).contains(&f) && (3..=4).contains(&r) { 12 } else { 0 },
        Kind::Knight => centre * 10 - 10,
        Kind::Bishop => centre * 5,
        Kind::Rook => if r == 6 { 15 } else { 0 },
        Kind::Queen => centre * 2,
        Kind::King => if r == 0 && (f <= 2 || f >= 6) { 20 } else { -r * 8 },
    }
}

/// The position from the side to move's view, in centipawns.
pub fn evaluate(pos: &Position) -> i32 {
    let mut s = 0;
    for (sq, p) in pos.board.iter().enumerate() {
        if let Some(p) = p {
            let v = value(p.kind) + square_bonus(*p, sq as u8);
            s += if p.color == pos.side { v } else { -v };
        }
    }
    s
}

const MATE: i32 = 100_000;

fn order(pos: &Position, moves: &mut [Move]) {
    moves.sort_by_key(|m| {
        let victim = pos.board[m.to as usize].map(|p| value(p.kind)).unwrap_or(0);
        let attacker = pos.board[m.from as usize].map(|p| value(p.kind)).unwrap_or(0);
        let promo = m.promotion.map(value).unwrap_or(0);
        -(victim * 10 - attacker / 10 + promo)
    });
}

fn quiesce(pos: &Position, mut alpha: i32, beta: i32, depth: u32, nodes: &mut u64) -> i32 {
    *nodes += 1;
    let stand = evaluate(pos);
    if stand >= beta || depth == 0 {
        return stand;
    }
    alpha = alpha.max(stand);
    let mut moves: Vec<Move> = pos.legal_moves().into_iter().filter(|m| pos.board[m.to as usize].is_some() || m.promotion.is_some()).collect();
    order(pos, &mut moves);
    for m in moves {
        let score = -quiesce(&pos.play(m), -beta, -alpha, depth - 1, nodes);
        if score >= beta {
            return score;
        }
        alpha = alpha.max(score);
    }
    alpha
}

fn search(pos: &Position, depth: u32, mut alpha: i32, beta: i32, ply: i32, nodes: &mut u64) -> i32 {
    *nodes += 1;
    let mut moves = pos.legal_moves();
    if moves.is_empty() {
        return if pos.in_check(pos.side) { -MATE + ply } else { 0 };
    }
    if depth == 0 {
        return quiesce(pos, alpha, beta, 4, nodes);
    }
    order(pos, &mut moves);
    let mut best = -MATE;
    for m in moves {
        let score = -search(&pos.play(m), depth - 1, -beta, -alpha, ply + 1, nodes);
        best = best.max(score);
        alpha = alpha.max(score);
        if alpha >= beta {
            break;
        }
    }
    best
}

/// The engine's move: `depth` plies of search; `seed` picks among moves that score the same, and
/// `slack` lets weaker play choose any move within that many centipawns of the best.
pub fn best_move(pos: &Position, depth: u32, seed: u64, slack: i32) -> Option<Move> {
    let mut moves = pos.legal_moves();
    order(pos, &mut moves);
    let mut nodes = 0;
    let mut scored: Vec<(i32, Move)> = Vec::new();
    let mut alpha = -MATE;
    for m in moves {
        let score = -search(&pos.play(m), depth.saturating_sub(1), -MATE, -alpha + slack + 1, 1, &mut nodes);
        alpha = alpha.max(score);
        scored.push((score, m));
    }
    let best = scored.iter().map(|s| s.0).max()?;
    let good: Vec<Move> = scored.iter().filter(|s| s.0 >= best - slack).map(|s| s.1).collect();
    good.get((seed % good.len() as u64) as usize).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perft_start() {
        let p = Position::start();
        assert_eq!(p.perft(1), 20);
        assert_eq!(p.perft(2), 400);
        assert_eq!(p.perft(3), 8902);
    }

    #[test]
    fn perft_kiwipete() {
        // Castling, en passant, promotions and pins all at once.
        let p = Position::from_fen("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1").unwrap();
        assert_eq!(p.perft(1), 48);
        assert_eq!(p.perft(2), 2039);
    }

    #[test]
    fn perft_endgame_and_promotions() {
        let p = Position::from_fen("8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1").unwrap();
        assert_eq!(p.perft(3), 2812);
        let p = Position::from_fen("r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1").unwrap();
        assert_eq!(p.perft(2), 264);
    }

    #[test]
    fn fen_round_trip_and_uci() {
        let p = Position::start();
        assert_eq!(p.fen(), START);
        let p = p.play(p.parse_move("e2e4").unwrap());
        assert_eq!(p.fen(), "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1");
        assert!(p.parse_move("e2e4").is_none());
    }

    #[test]
    fn scholars_mate_and_outcomes() {
        let mut p = Position::start();
        for m in ["e2e4", "e7e5", "f1c4", "b8c6", "d1h5", "g8f6", "h5f7"] {
            p = p.play(p.parse_move(m).unwrap());
        }
        assert_eq!(p.outcome(&[]), Some(Outcome::Checkmate(Color::White)));
        let stalemate = Position::from_fen("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1").unwrap();
        assert_eq!(stalemate.outcome(&[]), Some(Outcome::Stalemate));
        let bare = Position::from_fen("8/8/4k3/8/8/2N5/4K3/8 w - - 0 1").unwrap();
        assert_eq!(bare.outcome(&[]), Some(Outcome::InsufficientMaterial));
        let rook = Position::from_fen("8/8/4k3/8/8/2R5/4K3/8 w - - 0 1").unwrap();
        assert_eq!(rook.outcome(&[]), None);
    }

    #[test]
    fn engine_takes_free_material_and_mates() {
        // A queen hangs: take it.
        let p = Position::from_fen("4k3/8/8/3q4/8/8/3R4/4K3 w - - 0 1").unwrap();
        assert_eq!(best_move(&p, 2, 0, 0).unwrap().uci(), "d2d5");
        // Mate in one.
        let p = Position::from_fen("6k1/5ppp/8/8/8/8/8/R5K1 w - - 0 1").unwrap();
        assert_eq!(best_move(&p, 2, 0, 0).unwrap().uci(), "a1a8");
    }
}
