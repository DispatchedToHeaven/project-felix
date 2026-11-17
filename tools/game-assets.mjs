// Makes what the board games draw with:
// - textures/gui/pieces.png: 16x16 icons for the board screen (row 0 white chess K Q R B N P,
//   row 1 black chess, row 2 draughts: white man, white king, black man, black king);
// - geo/game_table.geo.json: the chess table without pieces (texture chess_table.png);
// - geo/game_pieces.geo.json: one bone per kind of piece, standing at the origin, for the table to
//   draw on any square (bones wk wq wr wb wn wp bk ... and w_man w_dking b_man b_dking for draughts;
//   bone names don't tell case apart).
// Usage: node tools/game-assets.mjs   (after tools/chess-table.mjs, whose texture they share)
import { writeFileSync, readFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { image, save } from './png.mjs';

const assets = (p) => fileURLToPath(new URL('../assets/projectfelix/' + p, import.meta.url));

// ---- icons: '#' outline, 'o' body ----
const GLYPHS = {
  k: ['       ##       ', '      #oo#      ', '     ##oo##     ', '     #oooo#     ', '      #oo#      ', '    ########    ', '   #oooooooo#   ', '   #oooooooo#   ', '    #oooooo#    ', '    #oooooo#    ', '     #oooo#     ', '    #oooooo#    ', '   #oooooooo#   ', '  #oooooooooo#  ', '  ############  ', '                '],
  q: ['  #   #  #   #  ', ' #o# #o##o# #o# ', '  #oo#oooo#oo#  ', '  #oooooooooo#  ', '   #oooooooo#   ', '   #oooooooo#   ', '    #oooooo#    ', '    #oooooo#    ', '    #oooooo#    ', '     #oooo#     ', '    #oooooo#    ', '   #oooooooo#   ', '  #oooooooooo#  ', '  #oooooooooo#  ', '  ############  ', '                '],
  r: ['                ', '  ### #### ###  ', '  #o# #oo# #o#  ', '  #oo##oo##oo#  ', '  #oooooooooo#  ', '   #oooooooo#   ', '    #oooooo#    ', '    #oooooo#    ', '    #oooooo#    ', '    #oooooo#    ', '    #oooooo#    ', '   #oooooooo#   ', '  #oooooooooo#  ', '  #oooooooooo#  ', '  ############  ', '                '],
  b: ['       ##       ', '      #oo#      ', '     #oooo#     ', '    #oo#ooo#    ', '    #ooo#oo#    ', '    #oooo#o#    ', '    #oooooo#    ', '     #oooo#     ', '     #oooo#     ', '    ########    ', '     #oooo#     ', '    #oooooo#    ', '   #oooooooo#   ', '  #oooooooooo#  ', '  ############  ', '                '],
  n: ['     ## ##      ', '    #oo#oo#     ', '   #oooooooo#   ', '  #oo#ooooooo#  ', '  #oooooooooo#  ', ' #ooooooooooo#  ', ' #oooo##ooooo#  ', '  ####  #oooo#  ', '       #ooooo#  ', '      #oooooo#  ', '     #ooooooo#  ', '    #oooooooo#  ', '   #oooooooooo# ', '   #oooooooooo# ', '   ############ ', '                '],
  p: ['                ', '                ', '       ##       ', '      #oo#      ', '     #oooo#     ', '     #oooo#     ', '      #oo#      ', '     #oooo#     ', '      #oo#      ', '      #oo#      ', '     #oooo#     ', '    #oooooo#    ', '   #oooooooo#   ', '   #oooooooo#   ', '   ##########   ', '                '],
  m: ['                ', '                ', '                ', '     ######     ', '   ##oooooo##   ', '  #oooooooooo#  ', ' #oooo####oooo# ', ' #ooo#oooo#ooo# ', ' #ooo#oooo#ooo# ', ' #oooo####oooo# ', ' ##oooooooooo## ', ' #o##oooooo##o# ', '  #oo######oo#  ', '   ##oooooo##   ', '     ######     ', '                '],
  K: ['  #   ##   #    ', ' #o# #oo# #o#   ', '  #oo#oo#oo#    ', '   ########     ', '   ##oooooo##   ', '  #oooooooooo#  ', ' #oooo####oooo# ', ' #ooo#oooo#ooo# ', ' #ooo#oooo#ooo# ', ' #oooo####oooo# ', ' ##oooooooooo## ', ' #o##oooooo##o# ', '  #oo######oo#  ', '   ##oooooo##   ', '     ######     ', '                '],
};
const ICON = 16;
const gui = image(6 * ICON, 3 * ICON);
gui.rect(0, 0, gui.w, gui.h, '#00000000');
function glyph(name, col, row, body, line, crown) {
  const g = GLYPHS[name];
  for (let y = 0; y < ICON; y++) for (let x = 0; x < ICON; x++) {
    const c = g[y][x];
    if (c === '#') gui.dot(col * ICON + x, row * ICON + y, line);
    else if (c === 'o') gui.dot(col * ICON + x, row * ICON + y, crown && y < 3 ? crown : body);
  }
}
'kqrbnp'.split('').forEach((k, i) => {
  const crown = k === 'k' || k === 'q' ? '#d4a72c' : undefined;
  glyph(k, i, 0, '#f6f1e4', '#2a2420', crown); glyph(k, i, 1, '#33302e', '#d8d0c0', crown);
});
glyph('m', 0, 2, '#f6f1e4', '#2a2420'); glyph('K', 1, 2, '#f6f1e4', '#2a2420', '#d4a72c');
glyph('m', 2, 2, '#33302e', '#d8d0c0'); glyph('K', 3, 2, '#33302e', '#d8d0c0', '#d4a72c');
mkdirSync(assets('textures/gui'), { recursive: true });
save(gui, assets('textures/gui/pieces.png'));

// ---- the empty table: the chess table's table and board bones ----
const chess = JSON.parse(readFileSync(assets('geo/chess_table.geo.json'), 'utf8'));
const g0 = chess['minecraft:geometry'][0];
const table = structuredClone(chess);
table['minecraft:geometry'][0].description.identifier = 'geometry.game_table';
table['minecraft:geometry'][0].bones = g0.bones.filter((b) => b.name === 'table' || b.name === 'board');
writeFileSync(assets('geo/game_table.geo.json'), JSON.stringify(table, null, '\t') + '\n');
console.log('wrote', assets('geo/game_table.geo.json'));

// ---- piece templates, standing at the origin ----
const face = (u, v) => ({ uv: [u, v], uv_size: [1, 1] });
const solid = (u, v) => Object.fromEntries(['north', 'south', 'east', 'west', 'up', 'down'].map((k) => [k, face(u, v)]));
const SHAPE = { p: [1, 1.6], r: [1.3, 2.2], n: [1.2, 2.4], b: [1.1, 2.7], q: [1.3, 3.1], k: [1.3, 3.4] };
const bones = [];
for (const color of ['w', 'b']) {
  const uv = color === 'w' ? solid(32, 0) : solid(36, 0);
  for (const kind of 'kqrbnp') {
    const [w, h] = SHAPE[kind];
    const cubes = [{ origin: [-w / 2, 0, -w / 2], size: [w, h, w], uv }];
    if (kind === 'k' || kind === 'q') cubes.push({ origin: [-0.3, h, -0.3], size: [0.6, 0.6, 0.6], uv: solid(40, 0) });
    if (kind === 'n') cubes.push({ origin: [-0.5, h - 0.8, color === 'w' ? -1.2 : 0.2], size: [1, 0.8, 1], uv });
    bones.push({ name: color + kind, pivot: [0, 0, 0], cubes });
  }
  bones.push({ name: color + '_man', pivot: [0, 0, 0], cubes: [{ origin: [-0.8, 0, -0.8], size: [1.6, 0.5, 1.6], uv }] });
  bones.push({ name: color + '_dking', pivot: [0, 0, 0], cubes: [
    { origin: [-0.8, 0, -0.8], size: [1.6, 0.5, 1.6], uv },
    { origin: [-0.8, 0.5, -0.8], size: [1.6, 0.5, 1.6], uv },
    { origin: [-0.3, 1.0, -0.3], size: [0.6, 0.3, 0.6], uv: solid(40, 0) },
  ] });
}
const pieces = {
  format_version: '1.21.0',
  'minecraft:geometry': [{ description: { identifier: 'geometry.game_pieces', texture_width: 64, texture_height: 64 }, bones }],
};
writeFileSync(assets('geo/game_pieces.geo.json'), JSON.stringify(pieces, null, '\t') + '\n');
console.log('wrote', assets('geo/game_pieces.geo.json'), bones.length, 'bones');
