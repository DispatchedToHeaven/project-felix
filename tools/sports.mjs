// Makes the ball games' props: a table tennis table, a tennis court with its net, a basketball
// hoop, and the three balls (geo/*.geo.json), on one texture (textures/entity/sports.png).
// Models are written in Bedrock's coordinates, as Blockbench exports them (x turned around
// against the game's), each standing on its own origin.
// Usage: node tools/sports.mjs
import { writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { image, save } from './png.mjs';

const assets = (p) => fileURLToPath(new URL('../assets/projectfelix/' + p, import.meta.url));

// One 2x2 swatch a colour: [u, v] of each.
const COLORS = {
  table: '#1f6b3a', white: '#f2f2f2', net: '#d8d8d8', grey: '#6d6d72', court: '#3a7a5a', courtOut: '#557a4a',
  wood: '#8a5a32', board: '#f4f4f4', rim: '#e05a1a', ping: '#fafafa', tennis: '#cfe23a', basket: '#d9701e', dark: '#2b2b30',
};
const img = image(64, 64);
img.rect(0, 0, 64, 64, '#00000000');
const UV = {};
Object.entries(COLORS).forEach(([k, c], i) => {
  const u = (i % 16) * 4, v = Math.floor(i / 16) * 4;
  img.rect(u, v, 4, 4, c);
  UV[k] = [u + 1, v + 1];
});
save(img, assets('textures/entity/sports.png'));

const solid = (k) => Object.fromEntries(['north', 'south', 'east', 'west', 'up', 'down'].map((f) => [f, { uv: UV[k], uv_size: [1, 1] }]));
const cube = (origin, size, color) => ({ origin, size, uv: solid(color) });
function model(name, bones) {
  const geo = { format_version: '1.21.0', 'minecraft:geometry': [{ description: { identifier: 'geometry.' + name, texture_width: 64, texture_height: 64 }, bones }] };
  writeFileSync(assets(`geo/${name}.geo.json`), JSON.stringify(geo, null, '\t') + '\n');
  console.log('wrote', name, bones.reduce((n, b) => n + (b.cubes?.length ?? 0), 0), 'cubes');
}

// Table tennis: 2.6 x 1.5 blocks, 12 texels high, along z; net across the middle.
const L = 41.6, W = 24;
model('table_tennis_table', [
  { name: 'table', pivot: [0, 0, 0], cubes: [
    cube([-W / 2, 11, -L / 2], [W, 1, L], 'table'),
    cube([-0.25, 12, -L / 2], [0.5, 0.05, L], 'white'),
    cube([-W / 2, 12, -L / 2], [W, 0.05, 0.4], 'white'),
    cube([-W / 2, 12, L / 2 - 0.4], [W, 0.05, 0.4], 'white'),
    ...[[-W / 2 + 1, -L / 2 + 2], [W / 2 - 3, -L / 2 + 2], [-W / 2 + 1, L / 2 - 4], [W / 2 - 3, L / 2 - 4]].map(([x, z]) => cube([x, 0, z], [2, 11, 2], 'grey')),
  ] },
  { name: 'net', parent: 'table', pivot: [0, 12, 0], cubes: [cube([-W / 2 - 1, 12, -0.2], [W + 2, 2.4, 0.4], 'net')] },
]);

// Tennis: a 5 x 10 block court (z from -5 to 5 blocks), its lines, a net in the middle.
const CW = 80, CL = 160;
const line = (x, z, w, l) => cube([x, 0.15, z], [w, 0.05, l], 'white');
model('tennis_court', [
  { name: 'court', pivot: [0, 0, 0], cubes: [
    cube([-CW / 2 - 8, 0, -CL / 2 - 12], [CW + 16, 0.1, CL + 24], 'courtOut'),
    cube([-CW / 2, 0.1, -CL / 2], [CW, 0.05, CL], 'court'),
    line(-CW / 2, -CL / 2, 0.6, CL), line(CW / 2 - 0.6, -CL / 2, 0.6, CL),
    line(-CW / 2, -CL / 2, CW, 0.6), line(-CW / 2, CL / 2 - 0.6, CW, 0.6),
    line(-0.3, -CL / 4, 0.6, CL / 2), line(-CW / 2, -CL / 4, CW, 0.6), line(-CW / 2, CL / 4 - 0.6, CW, 0.6),
  ] },
  { name: 'net', parent: 'court', pivot: [0, 0, 0], cubes: [
    cube([-CW / 2 - 4, 0, -0.5], [1, 17, 1], 'grey'), cube([CW / 2 + 3, 0, -0.5], [1, 17, 1], 'grey'),
    cube([-CW / 2 - 3, 2, -0.15], [CW + 6, 13, 0.3], 'net'),
    cube([-CW / 2 - 3, 15, -0.25], [CW + 6, 1, 0.5], 'white'),
  ] },
]);

// Basketball: a pole on the origin, the board and rim towards the game's -x (Bedrock's +x),
// the rim's centre 0.6 blocks out and 2.6 up.
model('hoop', [
  { name: 'hoop', pivot: [0, 0, 0], cubes: [
    cube([-1, 0, -1], [2, 46, 2], 'grey'),
    cube([1, 40, -0.5], [3.8, 1, 1], 'grey'),
    cube([4.8, 36, -8], [0.6, 12, 16], 'board'),
    cube([5.3, 39, -3], [0.1, 4, 6], 'dark'),
    cube([5.4, 41.2, -3.2], [7, 0.4, 0.4], 'rim'), cube([5.4, 41.2, 2.8], [7, 0.4, 0.4], 'rim'),
    cube([12.4, 41.2, -3.2], [0.4, 0.4, 6.4], 'rim'), cube([5.4, 41.2, -3.2], [0.4, 0.4, 6.4], 'rim'),
    cube([6, 37.5, -2.6], [6, 3.7, 0.1], 'net'), cube([6, 37.5, 2.5], [6, 3.7, 0.1], 'net'), cube([11.8, 37.5, -2.6], [0.1, 3.7, 5.2], 'net'),
  ] },
]);

// Balls, centred on their origins.
model('balls', [
  { name: 'table_tennis', pivot: [0, 0, 0], cubes: [cube([-0.4, -0.4, -0.4], [0.8, 0.8, 0.8], 'ping')] },
  { name: 'tennis', pivot: [0, 0, 0], cubes: [cube([-0.7, -0.7, -0.7], [1.4, 1.4, 1.4], 'tennis')] },
  { name: 'basketball', pivot: [0, 0, 0], cubes: [cube([-2, -2, -2], [4, 4, 4], 'basket'), cube([-2.05, -0.2, -2.05], [4.1, 0.4, 4.1], 'dark')] },
]);
