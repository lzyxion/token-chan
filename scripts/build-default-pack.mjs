// 기본 캐릭터 팩 빌드 — 원본 그림 8장을 **공통 캔버스·공통 배율·발 정렬**로 맞춰
// src/assets/pet-default/ 에 굽는다.
//
// 왜 필요한가: 펫은 캐릭터를 고정 상자(.cat 104×96)에 `object-fit: contain` 으로 넣는다.
// 그래서 장마다 캔버스 크기가 다르면 **장마다 다른 배율로 늘어나** 상태가 바뀔 때마다
// 캐릭터 크기가 들쭉날쭉해진다. 8장을 같은 캔버스에 담아야 상자가 똑같이 맞추고,
// 그림에 그려진 대로의 비례(누우면 낮고, 서면 높다)가 화면에 그대로 나온다.
//
// 규칙
//   · 불투명 영역(내용)만 남기고 원본 여백은 버린다 — 여백 크기가 장마다 달라 기준이 못 된다
//   · 배율은 **8장 공통** — 장마다 맞추면 누운 그림이 선 그림만큼 커진다
//   · 캔버스는 가장 큰 내용에 맞춘 하나 (가로 = 최대 내용 폭, 세로 = 최대 내용 높이)
//   · 각 그림은 **하단 중앙** — 발이 캔버스 바닥에 닿고 가로는 가운데
//
// 사용법: node scripts/build-default-pack.mjs <원본폴더> [긴변px]
import { PNG } from "pngjs";
import { readFileSync, writeFileSync, readdirSync } from "node:fs";
import { join, basename } from "node:path";

const SRC = process.argv[2];
const LONG = Number(process.argv[3] ?? 640);
if (!SRC) {
  console.error("사용법: node scripts/build-default-pack.mjs <원본폴더> [긴변px]");
  process.exit(1);
}
const OUT = new URL("../src/assets/pet-default/", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");

/** 알파가 있는 픽셀의 경계 상자 — 8 이하는 눈에 안 보이는 잔여물이라 버린다 */
function bbox(png) {
  let x0 = png.width, y0 = png.height, x1 = -1, y1 = -1;
  for (let y = 0; y < png.height; y++) {
    for (let x = 0; x < png.width; x++) {
      if (png.data[(png.width * y + x) * 4 + 3] > 8) {
        if (x < x0) x0 = x;
        if (x > x1) x1 = x;
        if (y < y0) y0 = y;
        if (y > y1) y1 = y;
      }
    }
  }
  return x1 < 0 ? null : { x0, y0, w: x1 - x0 + 1, h: y1 - y0 + 1 };
}

/** 면적 평균 축소. 알파를 곱해서 섞는다 — 안 그러면 투명 픽셀의 색(보통 검정)이
 *  가장자리에 배어 캐릭터 윤곽에 검은 테가 생긴다. */
function resample(src, box, dw, dh) {
  const out = Buffer.alloc(dw * dh * 4);
  const sx = box.w / dw, sy = box.h / dh;
  for (let y = 0; y < dh; y++) {
    const y0 = box.y0 + y * sy, y1 = box.y0 + (y + 1) * sy;
    for (let x = 0; x < dw; x++) {
      const x0 = box.x0 + x * sx, x1 = box.x0 + (x + 1) * sx;
      let r = 0, g = 0, b = 0, a = 0, n = 0;
      for (let iy = Math.floor(y0); iy < Math.ceil(y1); iy++) {
        for (let ix = Math.floor(x0); ix < Math.ceil(x1); ix++) {
          // 픽셀이 목적지 칸과 겹치는 넓이만큼만 센다 (경계에서 반 픽셀씩 걸린다)
          const cw = Math.min(ix + 1, x1) - Math.max(ix, x0);
          const ch = Math.min(iy + 1, y1) - Math.max(iy, y0);
          if (cw <= 0 || ch <= 0) continue;
          const w = cw * ch;
          const i = (src.width * iy + ix) * 4;
          const al = src.data[i + 3] / 255;
          r += src.data[i] * al * w;
          g += src.data[i + 1] * al * w;
          b += src.data[i + 2] * al * w;
          a += src.data[i + 3] * w;
          n += w;
        }
      }
      const o = (dw * y + x) * 4;
      if (n === 0 || a === 0) continue;
      const al = a / n / 255; // 평균 알파 (0..1)
      out[o] = Math.round(r / n / al);
      out[o + 1] = Math.round(g / n / al);
      out[o + 2] = Math.round(b / n / al);
      out[o + 3] = Math.round(a / n);
    }
  }
  return out;
}

const files = readdirSync(SRC).filter((f) => f.toLowerCase().endsWith(".png")).sort();
const items = files.map((f) => {
  const png = PNG.sync.read(readFileSync(join(SRC, f)));
  const box = bbox(png);
  if (!box) throw new Error(`${f}: 불투명 픽셀이 없습니다`);
  return { name: basename(f, ".png"), png, box };
});

const maxW = Math.max(...items.map((i) => i.box.w));
const maxH = Math.max(...items.map((i) => i.box.h));
const k = LONG / Math.max(maxW, maxH);
const CW = Math.round(maxW * k), CH = Math.round(maxH * k);
console.log(`공통 캔버스 ${CW}x${CH} (배율 ${k.toFixed(4)}, 최대 내용 ${maxW}x${maxH})`);

for (const { name, png, box } of items) {
  const dw = Math.max(1, Math.round(box.w * k));
  const dh = Math.max(1, Math.round(box.h * k));
  const scaled = resample(png, box, dw, dh);
  const canvas = new PNG({ width: CW, height: CH });
  canvas.data.fill(0);
  const ox = Math.round((CW - dw) / 2); // 가로 가운데
  const oy = CH - dh;                   // 발이 바닥에
  for (let y = 0; y < dh; y++) {
    scaled.copy(canvas.data, ((y + oy) * CW + ox) * 4, y * dw * 4, (y + 1) * dw * 4);
  }
  const buf = PNG.sync.write(canvas, { deflateLevel: 9 });
  writeFileSync(join(OUT, `${name}.png`), buf);
  console.log(`  ${name.padEnd(10)} ${box.w}x${box.h} → ${dw}x${dh} @ (${ox},${oy})  ${(buf.length / 1024).toFixed(0)}KB`);
}
