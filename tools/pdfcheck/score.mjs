// PDF milestone 1, step 9: how far our pages are from pdf.js's, worst first,
// with side-by-side PNGs of the worst for the by-eye pass (ours left).
// Both pictures are averaged over 3x3 cells first, so edge smoothing that
// differs a little does not count; a cell counts when a channel is off by
// more than 48. Runs where pdfjs-render.mjs ran (needs @napi-rs/canvas).
//
//   cd ~/pdfcheck/pdfjs && node ~/src/vergence/tools/pdfcheck/score.mjs ~/pdfcheck/ours ~/pdfcheck/pdfjs-out ~/pdfcheck/review [worst=80]
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";

const require = createRequire(path.join(process.cwd(), "x.js"));
const { createCanvas } = require("@napi-rs/canvas");
const [ours, theirs, review, worstArg] = process.argv.slice(2);
const worst = Number(worstArg || 80);
fs.mkdirSync(review, { recursive: true });

function readPpm(file) {
  const data = fs.readFileSync(file);
  let i = 0;
  const fields = [];
  while (fields.length < 4) {
    while (data[i] <= 32) i++;
    const s = i;
    while (data[i] > 32) i++;
    fields.push(data.subarray(s, i).toString());
  }
  return { w: Number(fields[1]), h: Number(fields[2]), px: data.subarray(i + 1) };
}

const CELL = 3;
function cells(img) {
  const cw = Math.floor(img.w / CELL), ch = Math.floor(img.h / CELL);
  const out = new Float32Array(cw * ch * 3);
  for (let y = 0; y < ch * CELL; y++)
    for (let x = 0; x < cw * CELL; x++) {
      const s = (y * img.w + x) * 3, d = (Math.floor(y / CELL) * cw + Math.floor(x / CELL)) * 3;
      out[d] += img.px[s]; out[d + 1] += img.px[s + 1]; out[d + 2] += img.px[s + 2];
    }
  for (let k = 0; k < out.length; k++) out[k] /= CELL * CELL;
  return { cw, ch, v: out };
}

const index = new Map(fs.readFileSync(path.join(ours, "index.tsv"), "utf8").trim().split("\n").map((l) => {
  const f = l.split("\t");
  return [`${f[0]}-p${f[1]}`, f[5]];
}));
const scores = [];
for (const [name, file] of index) {
  const theirsFile = path.join(theirs, `${name}.ppm`);
  if (!fs.existsSync(theirsFile)) continue;
  const a = readPpm(path.join(ours, `${name}.ppm`)), b = readPpm(theirsFile);
  if (a.w !== b.w || a.h !== b.h) { scores.push({ name, file, score: 100 }); continue; }
  const ca = cells(a), cb = cells(b);
  let off = 0;
  for (let k = 0; k < ca.v.length; k += 3)
    if (Math.abs(ca.v[k] - cb.v[k]) > 48 || Math.abs(ca.v[k + 1] - cb.v[k + 1]) > 48 || Math.abs(ca.v[k + 2] - cb.v[k + 2]) > 48) off++;
  scores.push({ name, file, score: (100 * off) / (ca.cw * ca.ch) });
}
scores.sort((x, y) => y.score - x.score);
fs.writeFileSync(path.join(review, "scores.tsv"), scores.map((s) => `${s.score.toFixed(3)}\t${s.name}\t${s.file}`).join("\n") + "\n");

// Side by side, each half at most 700 px wide.
for (const [rank, s] of scores.slice(0, worst).entries()) {
  const a = readPpm(path.join(ours, `${s.name}.ppm`)), b = readPpm(path.join(theirs, `${s.name}.ppm`));
  const k = Math.min(1, 700 / Math.max(a.w, b.w));
  const W = Math.ceil((a.w + b.w) * k) + 12, H = Math.ceil(Math.max(a.h, b.h) * k);
  const canvas = createCanvas(W, H), ctx = canvas.getContext("2d");
  ctx.fillStyle = "#c00"; ctx.fillRect(0, 0, W, H);
  let x0 = 0;
  for (const img of [a, b]) {
    const c = createCanvas(img.w, img.h), cx = c.getContext("2d"), d = cx.createImageData(img.w, img.h);
    for (let i = 0, j = 0; j < img.w * img.h * 3; i += 4, j += 3) { d.data[i] = img.px[j]; d.data[i + 1] = img.px[j + 1]; d.data[i + 2] = img.px[j + 2]; d.data[i + 3] = 255; }
    cx.putImageData(d, 0, 0);
    ctx.drawImage(c, x0, 0, img.w * k, img.h * k);
    x0 += img.w * k + 12;
  }
  fs.writeFileSync(path.join(review, `${String(rank + 1).padStart(3, "0")}-${s.name}.png`), canvas.toBuffer("image/png"));
}
const at = (q) => scores[Math.floor((scores.length - 1) * q)]?.score.toFixed(2);
console.log(`${scores.length} pages scored: median ${at(0.5)}%, 90% ${at(0.1)}%, 99% ${at(0.01)}% of cells off; over 5%: ${scores.filter((s) => s.score > 5).length}`);
