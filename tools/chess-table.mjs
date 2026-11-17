// Makes the chess table prop: its Bedrock model (geo/chess_table.geo.json), its texture
// (textures/entity/chess_table.png) and the game it plays (animations/chess_table.animation.json,
// Scholar's mate: 1. e4 e5 2. Bc4 Nc6 3. Qh5 Nf6 4. Qxf7#), as Blockbench would export them.
// Usage: node tools/chess-table.mjs
import { writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { image, save } from './png.mjs';

const assets = (p) => fileURLToPath(new URL('../assets/projectfelix/' + p, import.meta.url));
const SQ = 2; // texels a square
const TOP = 12.1; // the board's surface
// Bedrock coordinates: the a-file at +x and rank 1 at +z (the side of the NPC, who plays white).
const fileX = (f) => (3.5 - f) * SQ;
const rankZ = (r) => (3.5 - r) * SQ;
const sq = (name) => [name.charCodeAt(0) - 97, Number(name[1]) - 1];
const at = (name) => { const [f, r] = sq(name); return [fileX(f), rankZ(r)]; };

// ---- texture ----
const img = image(64, 64);
img.rect(0, 0, 64, 64, '#00000000');
for (let r = 0; r < 8; r++) for (let f = 0; f < 8; f++) img.rect(f * 2, r * 2, 2, 2, (f + r) % 2 ? '#e8d3a8' : '#6b4226');
img.rect(16, 0, 16, 16, '#8a5a32');
for (let y = 0; y < 16; y += 3) img.rect(16, y, 16, 1, '#7a4e2a');
img.rect(32, 0, 4, 4, '#f2ead8'); // white pieces
img.rect(36, 0, 4, 4, '#2b2b30'); // black pieces
img.rect(40, 0, 4, 4, '#c9a227'); // crowns
save(img, assets('textures/entity/chess_table.png'));

// ---- model ----
const face = (u, v, w, h) => ({ uv: [u, v], uv_size: [w, h] });
const solid = (u, v) => Object.fromEntries(['north', 'south', 'east', 'west', 'up', 'down'].map((k) => [k, face(u, v, 1, 1)]));
const wood = { north: face(16, 0, 16, 1), south: face(16, 0, 16, 1), east: face(16, 0, 16, 1), west: face(16, 0, 16, 1), up: face(16, 0, 16, 16), down: face(16, 0, 16, 16) };
const bones = [
  { name: 'table', pivot: [0, 0, 0], cubes: [
    { origin: [-10, 11, -10], size: [20, 1, 20], uv: wood },
    ...[[-9, -9], [7, -9], [-9, 7], [7, 7]].map(([x, z]) => ({ origin: [x, 0, z], size: [2, 11, 2], uv: wood })),
  ] },
  { name: 'board', parent: 'table', pivot: [0, 12, 0], cubes: [
    { origin: [-8, 12, -8], size: [16, 0.1, 16], uv: { up: face(0, 0, 16, 16), north: face(16, 0, 16, 1), south: face(16, 0, 16, 1), east: face(16, 0, 16, 1), west: face(16, 0, 16, 1) } },
  ] },
];
const SHAPE = { p: [1, 1.6], r: [1.3, 2.2], n: [1.2, 2.4], b: [1.1, 2.7], q: [1.3, 3.1], k: [1.3, 3.4] };
const back = ['r', 'n', 'b', 'q', 'k', 'b', 'n', 'r'];
const pieces = [];
for (let f = 0; f < 8; f++) {
  const file = String.fromCharCode(97 + f);
  pieces.push(['w', back[f], file + '1'], ['w', 'p', file + '2'], ['b', 'p', file + '7'], ['b', back[f], file + '8']);
}
for (const [color, kind, square] of pieces) {
  const [x, z] = at(square);
  const [w, h] = SHAPE[kind];
  const uv = color === 'w' ? solid(32, 0) : solid(36, 0);
  const cubes = [{ origin: [x - w / 2, TOP, z - w / 2], size: [w, h, w], uv }];
  if (kind === 'k' || kind === 'q') cubes.push({ origin: [x - 0.3, TOP + h, z - 0.3], size: [0.6, 0.6, 0.6], uv: solid(40, 0) });
  if (kind === 'n') cubes.push({ origin: [x - 0.5, TOP + h - 0.8, z - (color === 'w' ? 1.2 : -0.2)], size: [1, 0.8, 1], uv });
  bones.push({ name: `${color}_${kind}_${square}`, parent: 'board', pivot: [x, TOP, z], cubes });
}
const geo = {
  format_version: '1.21.0',
  'minecraft:geometry': [{
    description: { identifier: 'geometry.chess_table', texture_width: 64, texture_height: 64, visible_bounds_width: 2, visible_bounds_height: 1.5, visible_bounds_offset: [0, 0.5, 0] },
    bones,
  }],
};
writeFileSync(assets('geo/chess_table.geo.json'), JSON.stringify(geo, null, '\t') + '\n');
console.log('wrote', assets('geo/chess_table.geo.json'), bones.length, 'bones');

// ---- the game ----
const moves = [
  [1, 'w_p_e2', 'e2', 'e4'],
  [2, 'b_p_e7', 'e7', 'e5'],
  [3, 'w_b_f1', 'f1', 'c4'],
  [4, 'b_n_b8', 'b8', 'c6'],
  [5, 'w_q_d1', 'd1', 'h5'],
  [6, 'b_n_g8', 'g8', 'f6'],
  [7, 'w_q_d1', 'h5', 'f7', 'b_p_f7'],
];
const r3 = (v) => v.map((x) => Math.round(x * 1000) / 1000);
const tracks = {};
const sounds = {};
const at0 = {}; // where each moving piece's offset is, from its starting square
for (const [t, bone, from, to, captured] of moves) {
  const start = bone.split('_')[2];
  const [sx, sz] = at(start);
  const [fx, fz] = at(from);
  const [tx, tz] = at(to);
  const a = [fx - sx, 0, fz - sz], b = [tx - sx, 0, tz - sz];
  const mid = [(a[0] + b[0]) / 2, 2.5, (a[2] + b[2]) / 2];
  const key = (tracks[bone] ??= { position: {} }).position;
  key[(t - 0.6).toFixed(2)] = r3(a);
  key[(t - 0.3).toFixed(2)] = { post: r3(mid), lerp_mode: 'catmullrom' };
  key[t.toFixed(2)] = r3(b);
  at0[bone] = b;
  sounds[t.toFixed(2)] = { effect: 'minecraft:block.wood.place' };
  if (captured) tracks[captured] = { scale: { '0.0': [1, 1, 1], [(t - 0.05).toFixed(2)]: { pre: [1, 1, 1], post: [0, 0, 0] } } };
}
// The black king falls.
tracks['b_k_e8'] = { rotation: { '7.6': [0, 0, 0], '8.2': { vector: [0, 0, 85], easing: 'easeOutBounce' } } };
sounds['8.20'] = { effect: 'minecraft:entity.player.levelup' };
const anim = {
  format_version: '1.8.0',
  animations: {
    'animation.chess_table.scholars_mate': {
      loop: 'hold_on_last_frame',
      animation_length: 9,
      bones: tracks,
      sound_effects: sounds,
      particle_effects: { '8.2': { effect: 'minecraft:happy_villager' } },
    },
  },
};
writeFileSync(assets('animations/chess_table.animation.json'), JSON.stringify(anim, null, '\t') + '\n');
console.log('wrote', assets('animations/chess_table.animation.json'));
