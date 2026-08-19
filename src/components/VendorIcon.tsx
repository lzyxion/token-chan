import type { Source } from "../types";

/**
 * 벤더 로고 — `src/assets/vendors/<소스 id>.svg` 가 빌드에 포함된다.
 * 파일명이 곧 `Source` 값이라 벤더를 추가하면 파일만 넣으면 된다.
 *
 * SVG 를 JSX 에 직접 붙여 넣지 않고 `<img src=...>` 로 그린다. Antigravity 마크가
 * 내부 id 12개(`<mask id="SVGacqsTzKr">` 등)를 만들고 `url(#SVGacqsTzKr)` 로 참조하는데,
 * id 는 **HTML 문서 전체에서 공유**돼서 같은 SVG 를 두 번 인라인하면 두 번째 사본이
 * 첫 번째의 마스크·필터를 가리키게 된다 (브라우저는 `url(#id)` 를 문서의 첫 일치로 푼다).
 * 게이지와 패널에 동시에 그리는 구조라 실제로 겹친다. `<img>` 는 이미지마다 별도 문서라
 * id 가 새어 나가지 않고, 원본 멀티컬러도 그대로 살아난다.
 *
 * 대신 바깥 CSS 가 내부 path 에 닿지 않아 **마크 색은 못 바꾼다** — 파일에 적힌 색이
 * 그대로 나온다. 그래서 어두운 배경에 맞추는 일은 CSS 가 아니라 **파일 쪽에서** 한다
 * (codex 마크는 가운데를 뚫어 배경이 비치게 다시 그렸다).
 */
const files = import.meta.glob("../assets/vendors/*.svg", {
  eager: true,
  query: "?url",
  import: "default",
}) as Record<string, string>;

const ICONS: Partial<Record<Source, string>> = Object.fromEntries(
  Object.entries(files).map(([path, url]) => [
    (path.split("/").pop() ?? "").replace(/\.svg$/, ""),
    url,
  ]),
);

export default function VendorIcon({
  source,
  size = 14,
  className = "",
}: {
  source: Source;
  size?: number;
  className?: string;
}) {
  const url = ICONS[source];
  // 로고 파일이 없는 벤더는 색 점으로 떨어진다 — 자리와 의미는 그대로 유지된다
  if (!url) {
    return <span className={`vendor-icon fallback ${source} ${className}`} />;
  }
  return (
    <img
      className={`vendor-icon ${source} ${className}`}
      src={url}
      width={size}
      height={size}
      alt=""
      draggable={false}
    />
  );
}
