// Paints the test NPC's texture (assets/projectfelix/textures/entity/felix.png, 64x64) on the
// geometry's box-UV layout. Right side red, left side green, nose orange: in game they show at
// once whether sides, faces and mesh UVs land where Blockbench puts them.
// Usage: node tools/felix-texture.mjs
import { writeFileSync } from 'node:fs';
import { deflateSync } from 'node:zlib';
import { fileURLToPath } from 'node:url';

const W = 64, H = 64;
const px = new Uint8Array(W * H * 4);
const hex = (c) => [parseInt(c.slice(1, 3), 16), parseInt(c.slice(3, 5), 16), parseInt(c.slice(5, 7), 16), 255];
function rect(x, y, w, h, color) {
  const c = hex(color);
  for (let j = y; j < y + h; j++) for (let i = x; i < x + w; i++) px.set(c, (j * W + i) * 4);
}

// Head (box UV at 0,0): hair all round, face on the front (north), eyes and mouth.
rect(0, 0, 32, 16, '#5a3a22');
rect(8, 8, 8, 8, '#e0b08a');
rect(8, 8, 8, 2, '#5a3a22');
rect(9, 11, 2, 1, '#ffffff'); rect(10, 11, 1, 1, '#2a4a8a');
rect(13, 11, 2, 1, '#ffffff'); rect(13, 11, 1, 1, '#2a4a8a');
rect(10, 14, 4, 1, '#9a4a3a');
// Nose (poly mesh UVs 0..4 x 12..16, counted from the bottom: rows 48..52 from the top).
rect(0, 48, 4, 4, '#ff8a00');
// Body (16,16): blue shirt, lighter front with a white F.
rect(16, 16, 24, 16, '#2a4a9a');
rect(20, 20, 8, 12, '#3a6ad0');
rect(22, 22, 1, 6, '#ffffff'); rect(22, 22, 4, 1, '#ffffff'); rect(22, 24, 3, 1, '#ffffff');
// Right arm (40,16): red sleeve, hand; left arm (32,48): green sleeve, hand.
rect(40, 16, 16, 16, '#c0302a'); rect(44, 16, 4, 4, '#e0b08a'); rect(40, 28, 16, 4, '#e0b08a'); rect(48, 16, 4, 4, '#e0b08a');
rect(32, 48, 16, 16, '#2a9a3a'); rect(36, 48, 4, 4, '#e0b08a'); rect(32, 60, 16, 4, '#e0b08a'); rect(40, 48, 4, 4, '#e0b08a');
// Legs: right (0,16) and left (16,48), dark trousers with a red / green stripe on the front.
rect(0, 16, 16, 16, '#22263a'); rect(4, 20, 4, 2, '#c0302a');
rect(16, 48, 16, 16, '#22263a'); rect(20, 52, 4, 2, '#2a9a3a');
// Hat: brim top (0,32), brim underside (10,32), brim edge (20,32); crown (32,0); feather (56,0).
rect(0, 32, 10, 10, '#3a3a3a'); rect(0, 32, 10, 1, '#c0302a');
rect(10, 32, 10, 10, '#1a1a1a'); rect(10, 32, 1, 10, '#2a9a3a');
rect(20, 32, 10, 1, '#5a5a5a');
rect(32, 0, 24, 9, '#3a3a3a'); rect(32, 6, 24, 1, '#c09a2a');
rect(56, 0, 4, 7, '#f4f4f4');

// PNG: RGBA, 8 bits, no filter.
const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc = (buf) => { let c = 0xffffffff; for (const b of buf) c = crcTable[(c ^ b) & 255] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
function chunk(type, data) {
  const t = Buffer.from(type, 'ascii');
  const out = Buffer.alloc(12 + data.length);
  out.writeUInt32BE(data.length, 0); t.copy(out, 4); data.copy(out, 8);
  out.writeUInt32BE(crc(Buffer.concat([t, data])), 8 + data.length);
  return out;
}
const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(W, 0); ihdr.writeUInt32BE(H, 4); ihdr[8] = 8; ihdr[9] = 6;
const raw = Buffer.alloc(H * (W * 4 + 1));
for (let y = 0; y < H; y++) Buffer.from(px.buffer, y * W * 4, W * 4).copy(raw, y * (W * 4 + 1) + 1);
const png = Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
const out = fileURLToPath(new URL('../assets/projectfelix/textures/entity/felix.png', import.meta.url));
writeFileSync(out, png);
console.log('wrote', out);
