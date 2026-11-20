// PDF milestone 1, step 9: pdf.js's pictures of the pages in a pdfcheck index,
// for the by-eye comparison. A check tool on a dev box only, never shipped:
// it runs from a folder where pdfjs-dist and @napi-rs/canvas were installed
// with npm (docs/pdf-milestone-1.md, step 9).
//
//   cd ~/pdfcheck/pdfjs && node ~/src/vergence/tools/pdfcheck/pdfjs-render.mjs ~/pdfcheck/ours ~/pdfcheck/pdfjs-out [scale]
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";

const require = createRequire(path.join(process.cwd(), "x.js"));
const pdfjs = await import(path.join(path.dirname(require.resolve("pdfjs-dist/package.json")), "legacy/build/pdf.mjs"));
const { createCanvas } = require("@napi-rs/canvas");
const dist = path.dirname(require.resolve("pdfjs-dist/package.json"));

const [ours, out, scaleArg] = process.argv.slice(2);
const scale = Number(scaleArg || 1.5);
fs.mkdirSync(out, { recursive: true });
const rows = fs.readFileSync(path.join(ours, "index.tsv"), "utf8").trim().split("\n").map((l) => l.split("\t"));
const byFile = new Map();
for (const [key, page, , w, h, file, s] of rows) {
  if (!byFile.has(file)) byFile.set(file, []);
  byFile.get(file).push({ key, page: Number(page), w: Number(w), h: Number(h), scale: Number(s || scale) });
}

let done = 0, failed = 0;
for (const [file, pages] of byFile) {
  let doc;
  try {
    doc = await pdfjs.getDocument({
      data: new Uint8Array(fs.readFileSync(file)),
      standardFontDataUrl: path.join(dist, "standard_fonts") + "/",
      cMapUrl: path.join(dist, "cmaps") + "/",
      cMapPacked: true,
      isEvalSupported: false,
      verbosity: 0,
    }).promise;
  } catch (e) {
    failed += pages.length;
    console.log(`not opened  ${file}: ${e.message}`);
    continue;
  }
  for (const p of pages) {
    try {
      const page = await doc.getPage(p.page);
      const viewport = page.getViewport({ scale: p.scale });
      // Our size (rounded up); pdf.js draws into the same canvas size.
      const canvas = createCanvas(p.w, p.h);
      const ctx = canvas.getContext("2d");
      ctx.fillStyle = "#fff";
      ctx.fillRect(0, 0, p.w, p.h);
      await page.render({ canvasContext: ctx, viewport, background: "#fff" }).promise;
      const rgba = ctx.getImageData(0, 0, p.w, p.h).data;
      const ppm = Buffer.alloc(p.w * p.h * 3);
      for (let i = 0, j = 0; i < rgba.length; i += 4, j += 3) {
        ppm[j] = rgba[i]; ppm[j + 1] = rgba[i + 1]; ppm[j + 2] = rgba[i + 2];
      }
      fs.writeFileSync(path.join(out, `${p.key}-p${p.page}.ppm`), Buffer.concat([Buffer.from(`P6 ${p.w} ${p.h} 255\n`), ppm]));
      page.cleanup();
      done++;
    } catch (e) {
      failed++;
      console.log(`not drawn  page ${p.page}  ${file}: ${e.message}`);
    }
  }
  await doc.destroy();
}
console.log(`${done} pages drawn by pdf.js ${pdfjs.version}, ${failed} not`);
