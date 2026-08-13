import type { PetState } from "../types";

/**
 * 상황(상태 전이·클릭 반응) → 캐릭터 대사.
 * 상태 자체는 Pet 이 이미 설정(임계값·비활성 상태)을 반영해 계산하므로,
 * 사용자가 끈 상태는 여기까지 오지 않는다.
 *
 * 문구는 설정(speechLines)의 같은 키로 덮어쓸 수 있다. 문구 안 `{변수}` 자리에는
 * 표시 시점의 실제 값이 들어간다(interpolate) — 값별 규칙:
 *  · `|` 는 말풍선 안 줄바꿈
 *  · 값을 알 수 없는 변수(예: 공식 미터 미수신 시 {세션})가 든 줄은 통째로 생략
 *  · 모르는 변수 이름은 오타를 알아챌 수 있게 그대로 노출
 */

/** 상황 키 → 기본 문구. `enter.*` = 그 상태로 들어갈 때, `leave.*` = 빠져나올 때.
 *  `done`(작업 완료)은 **상태 전이가 아니라 세션 하나의 사건**이라 그 밖에 있다 —
 *  펫의 `working` 상태는 세 벤더를 통틀어 하나라도 돌면 켜지므로, 상태로는 "Codex 는
 *  아직 도는데 Claude 가 끝났다" 를 말할 수 없다. 자세한 건 Pet.tsx 의 완료 감시 참고.
 *  `resetNotify` 의 기본 문구는 러스트(monitor.rs)에 있다 — 여기 값은 편집기 안내용. */
export const DEFAULT_LINES: Record<string, string[]> = {
  "enter.exhausted": [
    "토큰이 바닥났어…",
    "더는 못 쓰겠어. 리셋을 기다리자",
    "완전 소진! 잠깐 쉬어가자",
    "한도 끝! 여기까지야",
    "텅 비었어… 충전이 필요해",
    "오늘은 여기까지. 수고했어!",
    "에너지 0. 리셋만 기다리는 중",
  ],
  "enter.alert": [
    "슬슬 한도가 보여…",
    "이번 블록 꽤 썼어. 조심조심!",
    "속도 좀 줄일까?",
    "어라, 생각보다 많이 썼는데?",
    "한도 가까워지는 중! 아껴 쓰자",
    "경고등 켜졌어. 남은 건 알뜰하게",
    "슬슬 페이스 조절할 시간이야",
  ],
  "enter.refreshed": [
    "새 블록 시작!",
    "충전 완료! 다시 달려보자",
    "리셋됐어. 마음껏 써도 돼",
    "짜잔, 새 블록이야!",
    "에너지 가득! 뭐부터 할까?",
    "깨끗하게 초기화됐어",
    "리필 완료! 준비됐어",
  ],
  "enter.working": [
    "작업 시작했어!",
    "타닥타닥… 하는 중",
    "열심히 해볼게!",
    "자, 시작해볼까?",
    "집중 모드 on!",
    "맡겨줘, 해볼게",
    "생각 중… 조금만 기다려",
  ],
  // 완료 문구는 **첫 줄이 늘 렌더되게** 짠다 — `{제목}` 은 모를 수 있고(그 줄은 통째로
  // 생략된다) `{걸린시간}`·`{벤더}` 는 완료 시점에 항상 아는 값이다. 모든 줄이 생략되면
  // 말풍선이 안 뜬다 = 끝난 걸 못 알린다.
  done: [
    "{제목} 끝났어!|{걸린시간} 걸렸어",
    "다 끝났다~ 수고했어!|{걸린시간} 만에 완료",
    "작업 완료! {걸린시간} 걸렸어",
    "{벤더} 쪽 일이 끝났어|{제목}",
    "짠, 다 했어!|{제목} · {걸린시간}",
    "끝! {걸린시간} 만이야",
    "무사히 마쳤어|{벤더} · {걸린시간}",
  ],
  "enter.sleep": [
    "슬슬 졸려…",
    "조용하네… 낮잠 잘래",
    "zzz…",
    "심심해… 눈 좀 붙일게",
    "아무도 없나? 자러 간다",
    "하암— 졸음이 밀려와",
    "잠깐 대기 모드로 갈게",
  ],
  "leave.sleep": [
    "오랜만이야!",
    "잘 잤다… 다시 해볼까?",
    "어, 왔구나!",
    "기다렸어! 뭐 할까?",
    "눈 떴어. 준비 완료!",
    "돌아왔네? 반가워",
    "하암— 이제 깼어",
  ],
  // 첫 줄은 summary 만 있으면 항상 채워지는 값으로 — 공식 미터가 없어도 말풍선이 비지 않게
  poke: [
    "오늘 {오늘토큰} · {오늘비용}|세션 {세션}% · 주간 {주간}%|리셋까지 {리셋}",
    "오늘은 {오늘토큰} 썼어!|세션 {세션}% · 주간 {주간}%|{리셋} 뒤에 리셋돼",
    "지금까지 {오늘비용} 어치|{모델} 쓰는 중|세션 게이지는 {세션}%",
    "불렀어? 오늘 {오늘토큰} · {오늘비용}|주간 한도는 {주간}%",
    "체크! 오늘 {오늘비용}|리셋까지 {리셋} 남았어",
    "오늘 누적 {오늘토큰}|세션 {세션}% 썼어. {리셋시각}에 리셋!",
  ],
  resetNotify: [
    "{분}분 뒤에 블록이 리셋돼! ({시각})",
    "{분}분만 버티면 리셋이야 ({시각})",
    "곧 리셋! {시각}에 새 블록이 열려",
    "{시각} 리셋까지 {분}분 남았어",
    "리셋 임박! {분}분 뒤에 충전돼 ({시각})",
  ],
};

/** 상황 키 → 직전에 고른 문구. 같은 상황이 연달아 같은 대사를 내지 않게 한다. */
const lastPicked = new Map<string, string>();

/** 후보 중 하나를 무작위로. key 를 주면 직전 문구를 후보에서 빼고 고른다
 *  (후보가 하나뿐이면 그대로 반복 — 뺄 게 없다). */
export function pick(lines: string[], key?: string): string | null {
  if (!lines.length) return null;
  const last = key != null ? lastPicked.get(key) : undefined;
  const pool = lines.filter((l) => l !== last);
  const from = pool.length ? pool : lines;
  const chosen = from[Math.floor(Math.random() * from.length)];
  if (key != null) lastPicked.set(key, chosen);
  return chosen;
}

/** 그 상황의 문구 후보 — 사용자 문구(공백 제외)가 있으면 그것, 없으면 기본 */
export function linesFor(
  key: string,
  overrides: Record<string, string[]> | undefined,
): string[] {
  const custom = overrides?.[key]?.map((l) => l.trim()).filter(Boolean);
  if (custom?.length) return custom;
  return DEFAULT_LINES[key] ?? [];
}

/** 대사가 없는 전이면 null. 반환값은 아직 `{변수}` 가 남은 템플릿 — interpolate 로 채운다 */
export function speechFor(
  prev: PetState,
  next: PetState,
  overrides?: Record<string, string[]>,
): string | null {
  // 진입 대사가 우선 — 경고/소진처럼 알려야 할 상황을 이탈 대사가 가리지 않게
  const enterKey = `enter.${next}`;
  const enter = linesFor(enterKey, overrides);
  if (enter.length) return pick(enter, enterKey);
  const leaveKey = `leave.${prev}`;
  const leave = linesFor(leaveKey, overrides);
  if (leave.length) return pick(leave, leaveKey);
  return null;
}

/** 기본 문구 위에 캐릭터 팩 문구를 상황별로 덮어쓴다 — 실질 문구(공백 아님)가 있는 키만.
 *  팩에서 비워 둔 상황은 기본 문구 → 내장 기본 순으로 폴백된다. */
export function mergeLines(
  base: Record<string, string[]> | undefined,
  over: Record<string, string[]> | undefined,
): Record<string, string[]> {
  const out = { ...(base ?? {}) };
  for (const [k, v] of Object.entries(over ?? {})) {
    if (v?.some((l) => l.trim())) out[k] = v;
  }
  return out;
}

/**
 * `{변수}` 를 실제 값으로 치환. vars 값이 null 이면 "지금은 모르는 값" —
 * 그 변수가 든 줄(`|` 구분)을 통째로 생략한다. vars 에 없는 이름은 그대로 둔다.
 * 남는 줄이 없으면 null (말풍선을 띄우지 않음).
 */
export function interpolate(
  phrase: string,
  vars: Record<string, string | null>,
): string | null {
  const out: string[] = [];
  for (const seg of phrase.split("|")) {
    let missing = false;
    const line = seg.replace(/\{([^{}]+)\}/g, (raw, name: string) => {
      const key = name.trim();
      if (!(key in vars)) return raw;
      const v = vars[key];
      if (v == null) {
        missing = true;
        return raw;
      }
      return v;
    });
    if (!missing && line.trim()) out.push(line.trim());
  }
  return out.length ? out.join("\n") : null;
}
