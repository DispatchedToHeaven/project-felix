// Draws the firearm item icons (16x16 pixel art) as PNG files:
//   node tools/gunart.mjs assets/projectfelix/textures/item
// Each icon is rows of letters; the letters are the colours below.
import fs from 'node:fs';
import zlib from 'node:zlib';

const pal = {
  '.': [0, 0, 0, 0],
  k: [27, 27, 31, 255],
  w: [84, 54, 28, 255],
  W: [134, 88, 46, 255],
  m: [74, 77, 84, 255],
  M: [158, 164, 172, 255],
  b: [201, 162, 58, 255],
  p: [230, 220, 192, 255],
  s: [176, 164, 132, 255],
  d: [38, 41, 46, 255],
  g: [75, 80, 88, 255],
  G: [128, 134, 144, 255],
  o: [61, 74, 54, 255],
  O: [96, 112, 84, 255],
  r: [176, 58, 46, 255],
  y: [205, 168, 66, 255],
  Y: [240, 215, 122, 255],
  c: [184, 112, 58, 255],
  l: [111, 183, 230, 255],
};

const art = {
  musket: [
    '................',
    '.............kM.',
    '............kMm.',
    '...........kMm..',
    '..........kMm...',
    '.........kMmk...',
    '........kMmW....',
    '.......kMmWk....',
    '......kbmWk.....',
    '.....kWbWk......',
    '....kWWWk.......',
    '...kWWwk........',
    '..kWWwk.........',
    '.kWwwk..........',
    '.kwwk...........',
    '..kk............',
  ],
  flintlock_pistol: [
    '................',
    '................',
    '................',
    '............kk..',
    '...........kMMk.',
    '..........kMmk..',
    '.........kMmk...',
    '........kMmk....',
    '.......kbmWk....',
    '......kWbWk.....',
    '.....kWWk.......',
    '....kWWk........',
    '....kWwk........',
    '....kwwk........',
    '.....kk.........',
    '................',
  ],
  blunderbuss: [
    '............kkk.',
    '...........kMMMk',
    '...........kMmMk',
    '..........kMmmk.',
    '.........kMmk...',
    '........kMmk....',
    '.......kMmWk....',
    '......kbmWk.....',
    '.....kWbWk......',
    '....kWWWk.......',
    '...kWWwk........',
    '..kWWwk.........',
    '.kWwwk..........',
    '.kwwk...........',
    '..kk............',
    '................',
  ],
  cartridge: [
    '................',
    '................',
    '................',
    '.........kk.....',
    '........kmMk....',
    '.......kpmmk....',
    '......kppsk.....',
    '.....kppsk......',
    '....kppsk.......',
    '...kspsk........',
    '...kpsk.........',
    '...ksk..........',
    '....k...........',
    '................',
    '................',
    '................',
  ],
  pistol: [
    '................',
    '................',
    '................',
    '................',
    '..........kkk...',
    '.........kGGgk..',
    '........kGggk...',
    '.......kGggk....',
    '......kgggdk....',
    '......kdgdk.....',
    '.....kdddk......',
    '.....kddk.......',
    '....kddk........',
    '....kdk.........',
    '.....k..........',
    '................',
  ],
  submachine_gun: [
    '................',
    '................',
    '................',
    '...........kk...',
    '..........kGgk..',
    '.........kGgk...',
    '........kGgdk...',
    '.......kGgdkk...',
    '......kggdkdk...',
    '.....kdgdkkdk...',
    '....kddkk..k....',
    '...kddk.........',
    '...kdk..........',
    '....k...........',
    '................',
    '................',
  ],
  assault_rifle: [
    '................',
    '.............kk.',
    '............kGk.',
    '...........kGgk.',
    '..........kGgk..',
    '.........kGggk..',
    '........kGgdkk..',
    '.......kGgdkdk..',
    '......kGgdkkdk..',
    '.....kogdk..k...',
    '....kOogk.......',
    '...kOook........',
    '..kOook.........',
    '.kOook..........',
    '.kook...........',
    '..kk............',
  ],
  shotgun: [
    '................',
    '.............kk.',
    '............kGk.',
    '...........kGgk.',
    '..........kGgk..',
    '.........kGgWk..',
    '........kGgWk...',
    '.......kGgWk....',
    '......kGgdk.....',
    '.....kWgdk......',
    '....kWWdk.......',
    '...kWWwk........',
    '..kWWwk.........',
    '.kWwwk..........',
    '.kwwk...........',
    '..kk............',
  ],
  sniper_rifle: [
    '..............kk',
    '.............kGk',
    '............kGgk',
    '.........kkkGgk.',
    '........kllkgk..',
    '........klGgk...',
    '.......kkGgdk...',
    '.......kGgdk....',
    '......kGgdk.....',
    '.....kogdkk.....',
    '....kOogkdk.....',
    '...kOook.k......',
    '..kOook.........',
    '.kOook..........',
    '.kook...........',
    '..kk............',
  ],
  pistol_rounds: [
    '................',
    '................',
    '................',
    '................',
    '....c....c......',
    '...kyk..kyk.....',
    '...kYk..kYk.....',
    '...kyk..kyk..c..',
    '...kyk..kyk.kyk.',
    '...kkk..kkk.kYk.',
    '............kyk.',
    '............kyk.',
    '............kkk.',
    '................',
    '................',
    '................',
  ],
  rifle_rounds: [
    '................',
    '................',
    '....c.....c.....',
    '....c.....c.....',
    '...kck...kck....',
    '...kyk...kyk....',
    '...kYk...kYk....',
    '...kyk...kyk....',
    '...kyk...kyk....',
    '...kyk...kyk....',
    '...kyk...kyk....',
    '...kkk...kkk....',
    '................',
    '................',
    '................',
    '................',
  ],
  shotgun_shells: [
    '................',
    '................',
    '................',
    '................',
    '...kkk....kkk...',
    '...krk....krk...',
    '...krk....krk...',
    '...krk....krk...',
    '...krk....krk...',
    '...kyk....kyk...',
    '...kYk....kYk...',
    '...kkk....kkk...',
    '................',
    '................',
    '................',
    '................',
  ],
};

const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc = (buf) => {
  let c = 0xffffffff;
  for (const b of buf) c = crcTable[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};
const chunk = (type, data) => {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type), data]);
  const sum = Buffer.alloc(4);
  sum.writeUInt32BE(crc(body));
  return Buffer.concat([len, body, sum]);
};

const out = process.argv[2];
fs.mkdirSync(out, { recursive: true });
for (const [name, rows] of Object.entries(art)) {
  if (rows.length !== 16 || rows.some((r) => r.length !== 16)) throw new Error(name + ': not 16x16');
  const raw = Buffer.alloc(16 * (1 + 16 * 4));
  rows.forEach((row, y) => {
    raw[y * 65] = 0;
    [...row].forEach((ch, x) => {
      const c = pal[ch];
      if (!c) throw new Error(name + ': colour ' + ch);
      raw.set(c, y * 65 + 1 + x * 4);
    });
  });
  const head = Buffer.alloc(13);
  head.writeUInt32BE(16, 0);
  head.writeUInt32BE(16, 4);
  head.set([8, 6, 0, 0, 0], 8);
  const png = Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', head), chunk('IDAT', zlib.deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
  fs.writeFileSync(`${out}/${name}.png`, png);
  console.log(name + '.png', png.length);
}
