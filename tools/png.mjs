// A small RGBA PNG writer and painter for the tools that make textures.
import { writeFileSync } from 'node:fs';
import { deflateSync } from 'node:zlib';

export function image(w, h) {
  const px = new Uint8Array(w * h * 4);
  const hex = (c) => [parseInt(c.slice(1, 3), 16), parseInt(c.slice(3, 5), 16), parseInt(c.slice(5, 7), 16), c.length > 7 ? parseInt(c.slice(7, 9), 16) : 255];
  return {
    w, h, px,
    rect(x, y, rw, rh, color) {
      const c = hex(color);
      for (let j = y; j < y + rh; j++) for (let i = x; i < x + rw; i++) px.set(c, (j * w + i) * 4);
    },
    dot(x, y, color) { px.set(hex(color), (y * w + x) * 4); },
  };
}

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

export function save(img, path) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(img.w, 0); ihdr.writeUInt32BE(img.h, 4); ihdr[8] = 8; ihdr[9] = 6;
  const raw = Buffer.alloc(img.h * (img.w * 4 + 1));
  for (let y = 0; y < img.h; y++) Buffer.from(img.px.buffer, y * img.w * 4, img.w * 4).copy(raw, y * (img.w * 4 + 1) + 1);
  writeFileSync(path, Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]));
  console.log('wrote', path);
}
