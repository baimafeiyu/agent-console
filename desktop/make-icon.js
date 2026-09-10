// 生成最小可用的 32x32 ICO（青绿底 + 白色圆环 + 中心点），避免外部依赖
const fs = require('fs');
const path = require('path');
const size = 32;
const px = Buffer.alloc(size * size * 4); // BGRA, bottom-up rows
const set = (x, y, b, g, r) => {
  const row = size - 1 - y; // bottom-up
  const i = (row * size + x) * 4;
  px[i] = b; px[i + 1] = g; px[i + 2] = r; px[i + 3] = 255;
};
const cx = 15.5, cy = 15.5;
for (let y = 0; y < size; y++) {
  for (let x = 0; x < size; x++) {
    const d = Math.hypot(x - cx, y - cy);
    if (d <= 15) set(x, y, 0x6f, 0x85, 0x16);          // #16856f 底
    if (d > 8 && d <= 12) set(x, y, 0xf7, 0xfd, 0xff); // 白环
    if (d <= 4) set(x, y, 0xf7, 0xfd, 0xff);           // 中心
  }
}
const maskRow = 4; // 32px @1bpp → 4 字节/行
const mask = Buffer.alloc(size * maskRow);
const bih = Buffer.alloc(40);
bih.writeUInt32LE(40, 0);
bih.writeInt32LE(size, 4);
bih.writeInt32LE(size * 2, 8); // XOR+AND 高度
bih.writeUInt16LE(1, 12);
bih.writeUInt16LE(32, 14);
bih.writeUInt32LE(0, 16);
bih.writeUInt32LE(px.length + mask.length, 20);
const img = Buffer.concat([bih, px, mask]);
const dir = Buffer.alloc(6);
dir.writeUInt16LE(0, 0);
dir.writeUInt16LE(1, 2);
dir.writeUInt16LE(1, 4);
const entry = Buffer.alloc(16);
entry[0] = size; entry[1] = size; entry[2] = 0; entry[3] = 0;
entry.writeUInt16LE(1, 4);
entry.writeUInt16LE(32, 6);
entry.writeUInt32LE(img.length, 8);
entry.writeUInt32LE(22, 12);
const out = path.join(__dirname, '..', 'src-tauri', 'icons');
fs.mkdirSync(out, { recursive: true });
fs.writeFileSync(path.join(out, 'icon.ico'), Buffer.concat([dir, entry, img]));
console.log('icon.ico written:', img.length + 22, 'bytes');
