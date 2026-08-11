//! 최소 protobuf wire-format 리더.
//!
//! Antigravity CLI 는 대화 DB 의 blob 을 protobuf 로 남기는데 `.proto` 를 배포하지
//! 않는다. 스키마 없이 읽어야 하므로 필드 **번호**로 직접 접근하는 리더만 둔다.
//! prost 같은 코드 생성 크레이트는 스키마가 있어야 쓸 수 있어 여기선 쓸 수 없다.
//!
//! wire format 자체는 안정된 규격이라 필드 번호만 맞으면 버전이 올라가도 읽힌다.
//! 모르는 필드는 조용히 건너뛴다 — 그게 protobuf 의 전방 호환 규칙이다.

/// 필드 하나의 값. 필요한 세 가지만 구분한다.
#[derive(Clone, Copy, Debug)]
pub enum Value<'a> {
    Varint(u64),
    /// 길이 지정 — 문자열·바이트·중첩 메시지가 전부 이 형태다
    Bytes(&'a [u8]),
    /// 고정폭 (32/64비트). 지금 읽는 필드 중엔 없지만 건너뛰려면 파싱은 해야 한다.
    Fixed(u64),
}

/// 디코딩된 메시지 한 개. 소유하지 않고 원본 바이트를 빌린다.
#[derive(Clone, Copy, Debug)]
pub struct Message<'a>(&'a [u8]);

impl<'a> Message<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Message(buf)
    }

    /// 필드를 순서대로 훑는다. 깨진 바이트를 만나면 그 지점에서 멈춘다 —
    /// 앞부분까지 읽은 값은 유효하므로 버리지 않는다.
    pub fn fields(&self) -> Fields<'a> {
        Fields { buf: self.0, pos: 0 }
    }

    /// 같은 번호의 필드 중 **마지막** 값. protobuf 는 뒤에 온 값이 이긴다.
    fn last(&self, field: u32) -> Option<Value<'a>> {
        self.fields().filter(|(n, _)| *n == field).map(|(_, v)| v).last()
    }

    pub fn varint(&self, field: u32) -> Option<u64> {
        match self.last(field)? {
            Value::Varint(v) | Value::Fixed(v) => Some(v),
            Value::Bytes(_) => None,
        }
    }

    /// varint 를 못 읽으면 0 — 카운터 필드는 없는 것과 0 이 같은 뜻이다.
    pub fn u64(&self, field: u32) -> u64 {
        self.varint(field).unwrap_or(0)
    }

    pub fn bytes(&self, field: u32) -> Option<&'a [u8]> {
        match self.last(field)? {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }

    /// UTF-8 이 아니면 None — 문자열 필드를 잘못 짚었다는 뜻이라 조용히 넘긴다.
    pub fn str(&self, field: u32) -> Option<&'a str> {
        std::str::from_utf8(self.bytes(field)?).ok()
    }

    /// 중첩 메시지. 내용 검증은 하지 않는다 (읽을 때 필드 단위로 걸러진다).
    pub fn msg(&self, field: u32) -> Option<Message<'a>> {
        Some(Message(self.bytes(field)?))
    }

    /// 반복 필드의 모든 중첩 메시지
    pub fn repeated(&self, field: u32) -> impl Iterator<Item = Message<'a>> + 'a {
        self.fields().filter_map(move |(n, v)| match v {
            Value::Bytes(b) if n == field => Some(Message(b)),
            _ => None,
        })
    }

    /// `1.9.10` 같은 경로를 한 번에 따라간다.
    pub fn path(&self, path: &[u32]) -> Option<Message<'a>> {
        let mut cur = *self;
        for &f in path {
            cur = cur.msg(f)?;
        }
        Some(cur)
    }
}

pub struct Fields<'a> {
    buf: &'a [u8],
    pos: usize,
}

/// varint 하나를 읽고 다음 위치를 돌려준다.
fn varint(buf: &[u8], mut pos: usize) -> Option<(u64, usize)> {
    let (mut out, mut shift) = (0u64, 0u32);
    loop {
        let b = *buf.get(pos)?;
        pos += 1;
        out |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some((out, pos));
        }
        shift += 7;
        // 64비트를 넘기면 깨진 데이터다
        if shift > 63 {
            return None;
        }
    }
}

impl<'a> Iterator for Fields<'a> {
    type Item = (u32, Value<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.buf.len() {
            return None;
        }
        let (key, pos) = varint(self.buf, self.pos)?;
        let field = (key >> 3) as u32;
        // 필드 번호 0 은 규격상 없다 → 여기부터는 메시지가 아니다
        if field == 0 {
            return None;
        }
        let (value, next) = match key & 7 {
            0 => {
                let (v, p) = varint(self.buf, pos)?;
                (Value::Varint(v), p)
            }
            1 => {
                let end = pos.checked_add(8)?;
                let b = self.buf.get(pos..end)?;
                (Value::Fixed(u64::from_le_bytes(b.try_into().ok()?)), end)
            }
            2 => {
                let (len, p) = varint(self.buf, pos)?;
                let end = p.checked_add(len as usize)?;
                (Value::Bytes(self.buf.get(p..end)?), end)
            }
            5 => {
                let end = pos.checked_add(4)?;
                let b = self.buf.get(pos..end)?;
                (Value::Fixed(u32::from_le_bytes(b.try_into().ok()?) as u64), end)
            }
            // 3/4 는 폐기된 group — 길이를 알 수 없어 더 못 읽는다
            _ => return None,
        };
        self.pos = next;
        Some((field, value))
    }
}

#[cfg(test)]
pub(crate) mod build {
    //! 테스트에서 실데이터와 같은 모양의 blob 을 만들기 위한 인코더.

    pub fn varint(mut v: u64) -> Vec<u8> {
        let mut out = vec![];
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return out;
            }
            out.push(b | 0x80);
        }
    }

    pub fn field_varint(field: u32, v: u64) -> Vec<u8> {
        let mut out = varint((field as u64) << 3);
        out.extend(varint(v));
        out
    }

    pub fn field_bytes(field: u32, b: &[u8]) -> Vec<u8> {
        let mut out = varint(((field as u64) << 3) | 2);
        out.extend(varint(b.len() as u64));
        out.extend_from_slice(b);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;

    #[test]
    fn reads_varints_strings_and_nested_messages() {
        let inner = [field_varint(1, 37_829), field_varint(4, 256_000)].concat();
        let buf = [
            field_varint(3, 1071),
            field_bytes(19, b"gemini-3.6-flash"),
            field_bytes(9, &inner),
        ]
        .concat();

        let m = Message::new(&buf);
        assert_eq!(m.varint(3), Some(1071));
        assert_eq!(m.str(19), Some("gemini-3.6-flash"));
        assert_eq!(m.path(&[9]).unwrap().u64(1), 37_829);
        assert_eq!(m.path(&[9]).unwrap().u64(4), 256_000);
        // 없는 필드
        assert_eq!(m.varint(7), None);
        assert_eq!(m.u64(7), 0);
    }

    #[test]
    fn repeated_fields_are_all_visible() {
        let buf = [
            field_bytes(1, &field_varint(3, 10)),
            field_bytes(1, &field_varint(3, 20)),
            field_bytes(1, &field_varint(3, 30)),
        ]
        .concat();
        let sum: u64 = Message::new(&buf).repeated(1).map(|m| m.u64(3)).sum();
        assert_eq!(sum, 60);
    }

    #[test]
    fn truncated_input_keeps_the_prefix_it_could_read() {
        // 실파일이 잘려 있어도 앞에서 읽은 값은 살아야 한다
        let full = [field_varint(1, 5), field_bytes(2, b"abcdefgh")].concat();
        let cut = &full[..full.len() - 3];
        let m = Message::new(cut);
        assert_eq!(m.varint(1), Some(5));
        assert_eq!(m.bytes(2), None, "잘린 길이 지정 필드는 버려야 함");
    }

    #[test]
    fn garbage_does_not_panic() {
        for bad in [&[0xff_u8][..], &[0x08][..], &[0x00, 0x00][..], &[0x0a, 0xff][..]] {
            let m = Message::new(bad);
            assert_eq!(m.fields().count(), 0);
        }
    }
}
