import type { CharacterImages } from "../types";

/** 내장 기본 팩 이미지 (번들에 포함). 파일명이 곧 상태 이름 —
 *  기본 팩은 저장소에 함께 들어 있으므로 idle 은 항상 존재한다.
 *  펫(렌더링)과 캐릭터 스튜디오(썸네일)가 함께 쓴다. */
const bundledFiles = import.meta.glob("../assets/pet-default/*.{png,webp,gif,apng,svg}", {
  eager: true,
  query: "?url",
  import: "default",
}) as Record<string, string>;

export const DEFAULT_PACK_IMAGES: CharacterImages | null = (() => {
  const map: CharacterImages = {};
  for (const [path, url] of Object.entries(bundledFiles)) {
    const file = path.split("/").pop() ?? "";
    map[file.replace(/\.[^.]+$/, "")] = url;
  }
  return map.idle ? map : null;
})();
