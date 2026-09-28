// nvm_info.rs -- Intel I225 / I226 (Foxville) NVM 固件镜像离线体检工具
//
// BUILD (零依赖，纯 Rust 标准库):
//     rustc -O -C target-feature=+crt-static -o nvm_info.exe nvm_info.rs
// 如需调试信息更小：
//     rustc -O -C strip=symbols -C target-feature=+crt-static -o nvm_info.exe nvm_info.rs
//
// 协议：MIT
// 作者：为「倍控 G31-1338 四口机 I225-V 固件升级」项目而写

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

const APP: &str = "nvm_info";
const VERSION: &str = "1.0 (rust)";

// ============================================================================
// 0. 平台相关：强制 Windows 控制台走 UTF-8，否则中文会按 GBK 输出成乱码
// ============================================================================

#[cfg(windows)]
extern "system" {
    fn SetConsoleOutputCP(code_page: u32) -> i32;
    fn SetConsoleCP(code_page: u32) -> i32;
}

fn set_console_utf8() {
    #[cfg(windows)]
    unsafe {
        SetConsoleOutputCP(65001);
        SetConsoleCP(65001);
    }
}

// ============================================================================
// 1. 显示宽度（中文等全角字符在等宽终端里占 2 列）
// ============================================================================

fn char_width(c: char) -> usize {
    let u = c as u32;
    if (0x1100..=0x115F).contains(&u) {
        2
    } else if (0x2E80..=0x303E).contains(&u) {
        2
    } else if (0x3041..=0x33FF).contains(&u) {
        2
    } else if (0x3400..=0x4DBF).contains(&u) {
        2
    } else if (0x4E00..=0x9FFF).contains(&u) {
        2
    } else if (0xA000..=0xA4CF).contains(&u) {
        2
    } else if (0xAC00..=0xD7A3).contains(&u) {
        2
    } else if (0xF900..=0xFAFF).contains(&u) {
        2
    } else if (0xFE30..=0xFE6F).contains(&u) {
        2
    } else if (0xFF00..=0xFF60).contains(&u) {
        2
    } else if (0xFFE0..=0xFFE6).contains(&u) {
        2
    } else if (0x20000..=0x3FFFD).contains(&u) {
        2
    } else {
        1
    }
}

fn display_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

fn pad(s: &str, width: usize) -> String {
    let w = display_width(s);
    let mut out = String::with_capacity(s.len() + width);
    out.push_str(s);
    if w < width {
        for _ in 0..(width - w) {
            out.push(' ');
        }
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if display_width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = char_width(c);
        if w + cw > max.saturating_sub(1) {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('~');
    out
}

// ============================================================================
// 2. MD5（标准库没有，手写一份）
// ============================================================================

const MD5_S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21,
    6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

const MD5_K: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
    0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
    0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
    0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];

fn md5_rotl(x: u32, n: u32) -> u32 {
    (x << n) | (x >> (32 - n))
}

struct Md5 {
    state: [u32; 4],
    buf: Vec<u8>,
    len: u64,
}

impl Md5 {
    fn new() -> Md5 {
        Md5 {
            state: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476],
            buf: Vec::with_capacity(64),
            len: 0,
        }
    }

    fn update(&mut self, data: &[u8]) {
        self.len += data.len() as u64;
        self.buf.extend_from_slice(data);
        while self.buf.len() >= 64 {
            let block: Vec<u8> = self.buf.drain(..64).collect();
            Self::process(&mut self.state, &block);
        }
    }

    fn finish(mut self) -> String {
        let bit_len = (self.len * 8) as u64;
        let mut tail = Vec::new();
        tail.push(0x80u8);
        // 补零到满足 (have + 1 + zeros + 8) % 64 == 0
        let have = self.buf.len();
        let need = (56isize - (have as isize + 1)).rem_euclid(64) as usize;
        tail.extend_from_slice(&vec![0u8; need]);
        tail.extend_from_slice(&bit_len.to_le_bytes());

        let mut padded: Vec<u8> = Vec::with_capacity(have + tail.len() + 64);
        padded.extend_from_slice(&self.buf);
        padded.extend_from_slice(&tail);
        let mut i = 0usize;
        while i + 64 <= padded.len() {
            Self::process(&mut self.state, &padded[i..i + 64]);
            i += 64;
        }

        let mut out = String::new();
        for w in self.state.iter() {
            out.push_str(&format!("{:02x}{:02x}{:02x}{:02x}",
                w & 0xff, (w >> 8) & 0xff, (w >> 16) & 0xff, (w >> 24) & 0xff));
        }
        out
    }

    fn process(state: &mut [u32; 4], block: &[u8]) {
        let mut m = [0u32; 16];
        for i in 0..16 {
            m[i] = u32::from_le_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        let (mut a, mut b, mut c, mut d) = (state[0], state[1], state[2], state[3]);
        for i in 0..64usize {
            let (f, g) = match i / 16 {
                0 => (((b & c) | ((!b) & d)), i),
                1 => (((d & b) | ((!d) & c)), (5 * i + 1) % 16),
                2 => ((b ^ c ^ d), (3 * i + 5) % 16),
                _ => ((c ^ (b | (!d))), (7 * i) % 16),
            };
            let tmp = a
                .wrapping_add(f)
                .wrapping_add(MD5_K[i])
                .wrapping_add(m[g]);
            let new_b = b.wrapping_add(md5_rotl(tmp, MD5_S[i]));
            a = d;
            d = c;
            c = b;
            b = new_b;
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
    }
}

fn md5_hex(data: &[u8]) -> String {
    let mut h = Md5::new();
    h.update(data);
    h.finish()
}

// ============================================================================
// 3. DEFLATE 解压（为了能直接读 zip / tar.gz，不引入外部 crate）
// ============================================================================

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bits: u64,
    nbits: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> BitReader<'a> {
        BitReader { data, pos: 0, bits: 0, nbits: 0 }
    }

    fn need(&mut self, n: u32) -> Result<(), String> {
        while self.nbits < n {
            if self.pos >= self.data.len() {
                return Err("数据流提前结束".to_string());
            }
            self.bits |= (self.data[self.pos] as u64) << self.nbits;
            self.pos += 1;
            self.nbits += 8;
        }
        Ok(())
    }

    fn bits(&mut self, n: u32) -> Result<u32, String> {
        if n == 0 {
            return Ok(0);
        }
        self.need(n)?;
        let v = (self.bits & ((1u64 << n) - 1)) as u32;
        self.bits >>= n;
        self.nbits -= n;
        Ok(v)
    }

    fn align(&mut self) {
        let drop = self.nbits % 8;
        self.bits >>= drop;
        self.nbits -= drop;
    }
}

struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn decode(&self, br: &mut BitReader) -> Result<u16, String> {
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;
        for len in 1..16usize {
            code |= br.bits(1)? as i32;
            let count = self.counts[len] as i32;
            if code - first < count {
                return Ok(self.symbols[(index + code - first) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("非法 Huffman 编码".to_string())
    }
}

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
// code length code 的读取顺序
const CL_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

fn build_huffman(lengths: &[u8]) -> Result<Huffman, String> {
    let mut counts = [0u16; 16];
    for &l in lengths.iter() {
        counts[l as usize] += 1;
    }
    counts[0] = 0;
    let mut offs = [0u16; 16];
    for len in 1..15usize {
        offs[len + 1] = offs[len] + counts[len];
    }
    let mut symbols = vec![0u16; lengths.len()];
    for (sym, &l) in lengths.iter().enumerate() {
        if l != 0 {
            symbols[offs[l as usize] as usize] = sym as u16;
            offs[l as usize] += 1;
        }
    }
    Ok(Huffman { counts, symbols })
}

fn inflate(data: &[u8], cap: usize) -> Result<Vec<u8>, String> {
    let mut br = BitReader::new(data);
    let mut out: Vec<u8> = Vec::new();

    loop {
        let last = br.bits(1)?;
        let btype = br.bits(2)?;

        match btype {
            0 => {
                // 未压缩块
                br.align();
                let len = br.bits(16)? as usize;
                let _nlen = br.bits(16)?;
                for _ in 0..len {
                    if br.pos >= br.data.len() {
                        return Err("stored 块数据不足".to_string());
                    }
                    out.push(br.data[br.pos]);
                    br.pos += 1;
                }
            }
            1 | 2 => {
                let (lit, dist) = if btype == 1 {
                    let mut l = vec![8u8; 288];
                    for i in 144..256 {
                        l[i] = 9;
                    }
                    for i in 256..280 {
                        l[i] = 7;
                    }
                    for i in 280..288 {
                        l[i] = 8;
                    }
                    let d = vec![5u8; 30];
                    (l, d)
                } else {
                    let hlit = br.bits(5)? as usize + 257;
                    let hdist = br.bits(5)? as usize + 1;
                    let hclen = br.bits(4)? as usize + 4;
                    let mut cl = [0u8; 19];
                    for i in 0..hclen {
                        cl[CL_ORDER[i]] = br.bits(3)? as u8;
                    }
                    let clh = build_huffman(&cl)?;
                    let mut lengths = Vec::with_capacity(hlit + hdist);
                    let total = hlit + hdist;
                    while lengths.len() < total {
                        let sym = clh.decode(&mut br)? as usize;
                        match sym {
                            0..=15 => lengths.push(sym as u8),
                            16 => {
                                if lengths.is_empty() {
                                    return Err("Huffman 重复码缺少前值".to_string());
                                }
                                let prev = *lengths.last().unwrap();
                                let rep = 3 + br.bits(2)? as usize;
                                for _ in 0..rep {
                                    lengths.push(prev);
                                }
                            }
                            17 => {
                                let rep = 3 + br.bits(3)? as usize;
                                for _ in 0..rep {
                                    lengths.push(0);
                                }
                            }
                            18 => {
                                let rep = 11 + br.bits(7)? as usize;
                                for _ in 0..rep {
                                    lengths.push(0);
                                }
                            }
                            _ => return Err("非法 code length symbol".to_string()),
                        }
                    }
                    if lengths.len() > total {
                        lengths.truncate(total);
                    }
                    let dl = lengths[hlit..].to_vec();
                    let ll = lengths[..hlit].to_vec();
                    (ll, dl)
                };

                let lh = build_huffman(&lit)?;
                let dh = build_huffman(&dist)?;

                loop {
                    let sym = lh.decode(&mut br)? as usize;
                    if sym < 256 {
                        out.push(sym as u8);
                    } else if sym == 256 {
                        break;
                    } else {
                        let idx = sym - 257;
                        if idx >= 29 {
                            return Err("非法长度符号".to_string());
                        }
                        let len = LEN_BASE[idx] as usize + br.bits(LEN_EXTRA[idx] as u32)? as usize;
                        let dsym = dh.decode(&mut br)? as usize;
                        if dsym >= 30 {
                            return Err("非法距离符号".to_string());
                        }
                        let dist = DIST_BASE[dsym] as usize
                            + br.bits(DIST_EXTRA[dsym] as u32)? as usize;
                        if dist > out.len() {
                            return Err("距离超出已输出窗口".to_string());
                        }
                        let start = out.len() - dist;
                        for k in 0..len {
                            let b = out[start + k];
                            out.push(b);
                        }
                    }
                    if out.len() > cap {
                        return Err("解压结果超出上限".to_string());
                    }
                }
            }
            _ => return Err("不支持的 deflate 块类型 (BTYPE=3)".to_string()),
        }

        if out.len() > cap {
            return Err("解压结果超出上限".to_string());
        }
        if last == 1 {
            break;
        }
    }
    Ok(out)
}

// gzip：只处理单成员
fn gunzip(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 18 || data[0] != 0x1f || data[1] != 0x8b {
        return Err("不是 gzip 数据".to_string());
    }
    if data[2] != 8 {
        return Err("gzip 压缩方法不是 deflate".to_string());
    }
    let flg = data[3];
    let mut p: usize = 10;
    if flg & 0x04 != 0 {
        // FEXTRA
        if p + 2 > data.len() {
            return Err("gzip 头损坏".to_string());
        }
        let xlen = u16::from_le_bytes([data[p], data[p + 1]]) as usize;
        p += 2 + xlen;
    }
    for &mask in [0x08u8, 0x10, 0x02].iter() {
        if flg & mask != 0 {
            while p < data.len() && data[p] != 0 {
                p += 1;
            }
            p += 1;
        }
    }
    if p >= data.len() {
        return Err("gzip 头损坏".to_string());
    }
    inflate(&data[p..], 256 * 1024 * 1024)
}

// ============================================================================
// 4. 压缩包读取：zip / tar.gz（够用的最小实现，不支持 zip64 / 加密）
// ============================================================================

fn u32le_at(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}
fn u16le_at(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}

struct Entry {
    name: String,
    source: String,
    data: Vec<u8>,
}

fn read_zip(path: &str, data: &[u8]) -> Vec<Entry> {
    let mut out = Vec::new();
    // 找 End Of Central Directory
    let search_from = data.len().saturating_sub(66 * 1024);
    let mut eocd: Option<usize> = None;
    let mut i = data.len() - 4;
    while i >= search_from {
        if u32le_at(data, i) == 0x06054b50 {
            eocd = Some(i);
            break;
        }
        i = i.saturating_sub(1);
        if i == 0 {
            break;
        }
    }
    let eocd = match eocd {
        Some(v) => v,
        None => return out,
    };
    let count = u16le_at(data, eocd + 10) as usize;
    let mut off = u32le_at(data, eocd + 16) as usize;

    for _ in 0..count {
        if off + 46 > data.len() || u32le_at(data, off) != 0x02014b50 {
            break;
        }
        let method = u16le_at(data, off + 10);
        let comp_size = u32le_at(data, off + 20) as usize;
        let raw_name_len = u16le_at(data, off + 28) as usize;
        let extra_len = u16le_at(data, off + 30) as usize;
        let comment_len = u16le_at(data, off + 32) as usize;
        let lho = u32le_at(data, off + 42) as usize;
        let name = String::from_utf8_lossy(&data[off + 46..off + 46 + raw_name_len]).to_string();
        off += 46 + raw_name_len + extra_len + comment_len;

        let base = name.rsplit('/').next().unwrap_or(&name).to_string();
        if !is_image_name(&base) {
            continue;
        }
        if lho + 30 > data.len() || u32le_at(data, lho) != 0x04034b50 {
            continue;
        }
        let lname_len = u16le_at(data, lho + 26) as usize;
        let lextra_len = u16le_at(data, lho + 28) as usize;
        let dstart = lho + 30 + lname_len + lextra_len;
        let dend = (dstart + comp_size).min(data.len());
        let raw = &data[dstart..dend];

        let payload = match method {
            0 => Ok(raw.to_vec()),
            8 => inflate(raw, 256 * 1024 * 1024),
            _ => Err(format!("不支持的压缩方法 {}", method)),
        };
        match payload {
            Ok(bytes) => out.push(Entry {
                name: base,
                source: format!("{}::{}", path, name),
                data: bytes,
            }),
            Err(e) => println!("[!] 跳过 {} ({})", base, e),
        }
    }
    out
}

fn read_tar(path: &str, data: &[u8]) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut p = 0usize;
    while p + 512 <= data.len() {
        let name_bytes = &data[p..p + 100];
        if name_bytes[0] == 0 {
            break;
        }
        let name = String::from_utf8_lossy(name_bytes)
            .trim_matches(|c| c == '\0' || c == ' ')
            .to_string();
        let typeflag = data[p + 156];
        let size_str = String::from_utf8_lossy(&data[p + 124..p + 136])
            .trim_matches(|c| c == '\0' || c == ' ')
            .to_string();
        let size: usize = size_str.parse().unwrap_or(0);
        let dstart = p + 512;
        let dend = (dstart + size).min(data.len());
        let base = name.rsplit('/').next().unwrap_or(&name).to_string();

        if typeflag == b'0' || typeflag == 0 {
            if is_image_name(&base) {
                out.push(Entry {
                    name: base,
                    source: format!("{}::{}", path, name),
                    data: data[dstart..dend].to_vec(),
                });
            }
        }
        let blocks = (size + 511) / 512;
        p = dstart + blocks * 512;
    }
    out
}

fn is_image_name(n: &str) -> bool {
    let lower = n.to_ascii_lowercase();
    lower.ends_with(".bin") || lower.ends_with(".img") || lower.ends_with(".eep")
        || lower.ends_with(".dat")
}

// ============================================================================
// 5. 核心：解析 Foxville NVM 镜像头
// ============================================================================

struct ImageInfo {
    name: String,
    source: String,
    size: usize,
    md5: String,
    ok: bool,
    flash_idx: u8,
    flash_label: String,
    imgtype: u16,
    imgtype_label: String,
    nvmver: u16,
    nvmver_label: String,
    mac: String,
    vendor: u16,
    devid: u16,
    devid_label: String,
    subvendor: u16,
    subdevice: u16,
    eepid: u32,
    eepid_note: String,
    notes: Vec<String>,
    data: Vec<u8>,
}

fn flash_idx_label(b: u8) -> String {
    match b {
        0x0D => "1MB".to_string(),
        0x05 => "2MB".to_string(),
        _ => "未知".to_string(),
    }
}

fn imgtype_label(t: u16) -> String {
    match t {
        0x8022 => "1MB".to_string(),
        0x80A2 => "2MB".to_string(),
        _ => "未知".to_string(),
    }
}

fn devid_label(d: u16) -> String {
    match d {
        0x15F3 => "I225-V".to_string(),
        0x15F2 => "I225-LM".to_string(),
        0x15F8 => "I225-IT(未验证)".to_string(),
        0x125C => "I226-V".to_string(),
        0x125D => "I226-LM".to_string(),
        _ => "未知".to_string(),
    }
}

fn known_eepid(e: u32) -> String {
    match e {
        0x800003FC => "Foxpond1_I225_15F3_V_1MB_1p94（15F3 / 1MB / NVM 1.94）".to_string(),
        0x800002FC => "FXVL_15F3_V_1MB_1.89（15F3 / 1MB / NVM 1.89，Vendor 0x17AA）".to_string(),
        0x800002F4 => "FXVL_15F3_V_2MB_1.89（15F3 / 2MB / NVM 1.89）".to_string(),
        0x80000182 => "倍控 G31-1338 出厂备份（15F3 / 1MB / NVM 1.57）".to_string(),
        0x800003BB => "Intel 官方 FoxPond1_I225_15F2_2MB_1p94".to_string(),
        0x800003BC => "Intel 官方 Foxpond1_I225_15F2_LM_1MB_1p94".to_string(),
        _ => String::new(),
    }
}

/// 版本字的解码：实测规律是 word = 0x1000 + BCD(小版本)，
/// 即 0x1094 -> 0x94 -> BCD 94 -> "1.94"。千万别当成十进制除以 100。
fn nvm_version_label(w: u16) -> String {
    let hi = (w >> 8) as u8;
    let lo = (w & 0xff) as u8;
    if hi == 0x10 && (lo >> 4) <= 9 && (lo & 0x0f) <= 9 {
        format!("1.{:02}", (lo >> 4) * 10 + (lo & 0x0f))
    } else {
        "?".to_string()
    }
}

fn parse_image(name: &str, source: &str, data: Vec<u8>) -> ImageInfo {
    let mut r = ImageInfo {
        name: name.to_string(),
        source: source.to_string(),
        size: data.len(),
        md5: md5_hex(&data),
        ok: true,
        flash_idx: 0,
        flash_label: String::new(),
        imgtype: 0,
        imgtype_label: String::new(),
        nvmver: 0,
        nvmver_label: String::new(),
        mac: String::new(),
        vendor: 0,
        devid: 0,
        devid_label: String::new(),
        subvendor: 0,
        subdevice: 0,
        eepid: 0,
        eepid_note: String::new(),
        notes: Vec::new(),
        data,
    };

    if r.data.len() < 0x88 {
        r.ok = false;
        r.notes
            .push("文件太小（不足 0x88 字节），不像 Foxville NVM 镜像".to_string());
        return r;
    }

    let d = &r.data;
    r.flash_idx = d[0x07];
    r.flash_label = flash_idx_label(d[0x07]);
    r.imgtype = u16le_at(d, 0x20);
    r.imgtype_label = imgtype_label(r.imgtype);
    r.nvmver = u16le_at(d, 0x0A);
    r.nvmver_label = nvm_version_label(r.nvmver);
    r.mac = (0..6)
        .map(|i| format!("{:02X}", d[i]))
        .collect::<Vec<String>>()
        .join(":");
    r.vendor = u16le_at(d, 0x18);
    r.devid = u16le_at(d, 0x1A);
    r.devid_label = devid_label(r.devid);
    r.subvendor = u16le_at(d, 0x1C);
    r.subdevice = u16le_at(d, 0x1E);
    r.eepid = u32le_at(d, 0x84);
    r.eepid_note = known_eepid(r.eepid);

    let d = &r.data;
    if d[0x07] != 0x0D && d[0x07] != 0x05 {
        r.notes.push(format!(
            "0x07=0x{:02X} 不在已知容量索引表里，确认是不是 Foxville 镜像",
            d[0x07]
        ));
    }
    if r.imgtype != 0x8022 && r.imgtype != 0x80A2 {
        r.notes.push(format!(
            "0x20=0x{:04X} 不是已知镜像类型（1MB=0x8022 / 2MB=0x80A2）",
            r.imgtype
        ));
    }
    if flash_idx_label(d[0x07]) != "未知"
        && imgtype_label(r.imgtype) != "未知"
        && flash_idx_label(d[0x07]) != imgtype_label(r.imgtype)
    {
        r.notes
            .push("0x07 与 0x20 指向的容量不一致，头部可能损坏".to_string());
    }
    if !matches!(r.vendor, 0x8086 | 0x17AA | 0x1028 | 0x8087) {
        r.notes.push(format!(
            "0x18 Vendor=0x{:04X} 非 0x8086，可能不是 Intel NVM 镜像",
            r.vendor
        ));
    }
    if !matches!(r.devid, 0x15F3 | 0x15F2 | 0x125C | 0x125D) {
        r.notes.push(format!(
            "0x1A DeviceID=0x{:04X} 不是已知的 Foxville ID（15F3/15F2/125C/125D）",
            r.devid
        ));
    }

    // 2MB dump 的真相：是不是同一份 1MB 被写了两遍
    if r.data.len() == 2 * 1024 * 1024 {
        if r.data[..0x100000] == r.data[0x100000..] {
            r.notes.push(
                "前 1MB 与后 1MB 逐字节完全相同 -> 这是同一份 1MB 镜像被 dump 了两遍，\
                 不是四口各占一片；刷机请选 1MB 镜像"
                    .to_string(),
            );
        } else {
            r.notes
                .push("前 1MB 与后 1MB 内容不同，确为真实的 2MB 结构".to_string());
        }
    }

    r
}

// ============================================================================
// 6. 输入收集
// ============================================================================

fn collect_from_dir(dir: &Path, recursive: bool) -> Vec<Entry> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if recursive {
                    out.extend(collect_from_dir(&p, true));
                }
            } else {
                let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                if is_image_name(&name) {
                    if let Ok(data) = fs::read(&p) {
                        out.push(Entry {
                            name,
                            source: p.to_string_lossy().to_string(),
                            data,
                        });
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn collect(targets: &[String], recursive: bool) -> Vec<Entry> {
    let mut out = Vec::new();
    for t in targets {
        let path = PathBuf::from(t);
        if path.is_dir() {
            let got = collect_from_dir(&path, recursive);
            if got.is_empty() {
                println!("[!] 目录里没有 .bin/.img/.eep/.dat：{}", t);
            }
            out.extend(got);
        } else if path.is_file() {
            let Ok(raw) = fs::read(&path) else {
                println!("[!] 读取失败：{}", t);
                continue;
            };
            let s = t.to_ascii_lowercase();
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if s.ends_with(".zip") {
                let got = read_zip(t, &raw);
                if got.is_empty() {
                    println!("[!] zip 里没找到镜像文件：{}", t);
                }
                out.extend(got);
            } else if s.ends_with(".tar.gz") || s.ends_with(".tgz") || s.ends_with(".gz") {
                match gunzip(&raw) {
                    Ok(plain) => {
                        let got = read_tar(t, &plain);
                        if got.is_empty() {
                            println!("[!] tar 里没找到镜像文件：{}", t);
                        }
                        out.extend(got);
                    }
                    Err(e) => println!("[!] gzip 解压失败 {} ({})", name, e),
                }
            } else if s.ends_with(".tar") {
                out.extend(read_tar(t, &raw));
            } else {
                out.push(Entry {
                    name,
                    source: t.clone(),
                    data: raw,
                });
            }
        } else {
            println!("[!] 路径不存在：{}", t);
        }
    }
    out
}

// ============================================================================
// 7. 输出
// ============================================================================

fn line() -> String {
    "=".repeat(72)
}

fn thin_line() -> String {
    "-".repeat(72)
}

fn show_result(r: &ImageInfo) {
    println!("{}", line());
    println!("文件    : {}", r.name);
    if r.source != r.name {
        println!("来源    : {}", r.source);
    }
    println!("大小    : {} 字节 ({})", r.size, human_size(r.size));
    println!("MD5     : {}", r.md5);
    if !r.ok {
        for n in &r.notes {
            println!("[x] {}", n);
        }
        return;
    }
    println!("{}", thin_line());
    println!("MAC 地址    @0x00 : {}", r.mac);
    println!(
        "闪存索引    @0x07 : 0x{:02X}  -> {}",
        r.flash_idx, r.flash_label
    );
    println!(
        "NVM 版本    @0x0A : 0x{:04X}  -> {}",
        r.nvmver, r.nvmver_label
    );
    println!("Vendor ID   @0x18 : 0x{:04X}", r.vendor);
    println!(
        "Device ID   @0x1A : 0x{:04X}  -> {}",
        r.devid, r.devid_label
    );
    println!(
        "Subsystem   @0x1C : {:04X}:{:04X}",
        r.subvendor, r.subdevice
    );
    println!(
        "镜像类型    @0x20 : 0x{:04X}  -> {}",
        r.imgtype, r.imgtype_label
    );
    println!("EEPID/Etrack@0x84 : 0x{:08X}", r.eepid);
    if !r.eepid_note.is_empty() {
        println!("             已知  : {}", r.eepid_note);
    }
    if !r.notes.is_empty() {
        println!("{}", thin_line());
        for n in &r.notes {
            println!("[!] {}", n);
        }
    }
}

fn human_size(n: usize) -> String {
    match n {
        1048576 => "1MB".to_string(),
        2097152 => "2MB".to_string(),
        _ => format!("{:.2} MB", n as f64 / 1048576.0),
    }
}

fn show_compare(rs: &[ImageInfo]) {
    let ok: Vec<&ImageInfo> = rs.iter().filter(|r| r.ok).collect();
    if ok.len() < 2 {
        return;
    }
    println!();
    println!("{}", line());
    println!("汇总对照");
    println!("{}", line());

    let heads: Vec<String> = ok.iter().map(|r| truncate(&r.name, 26)).collect();
    let mut width = 14usize;
    for h in &heads {
        width = width.max(display_width(h) + 2);
    }
    let mut header = pad("", 12);
    for h in &heads {
        header.push_str(&pad(h, width));
    }
    println!("{}", header);

    type Getter = fn(&ImageInfo) -> String;
    let rows: Vec<(&str, Getter)> = vec![
        ("大小", |r| format!("{} ({})", r.size, human_size(r.size))),
        ("MAC", |r| r.mac.clone()),
        ("闪存 0x07", |r| format!("0x{:02X} {}", r.flash_idx, r.flash_label)),
        ("类型 0x20", |r| format!("0x{:04X} {}", r.imgtype, r.imgtype_label)),
        ("NVM 0x0A", |r| format!("0x{:04X} {}", r.nvmver, r.nvmver_label)),
        ("DevID 0x1A", |r| format!("0x{:04X}", r.devid)),
        ("EEPID 0x84", |r| format!("0x{:08X}", r.eepid)),
    ];

    for (label, getter) in rows {
        let vals: Vec<String> = ok.iter().map(|r| getter(r)).collect();
        let same = vals.windows(2).all(|w| w[0] == w[1]);
        let mut row = pad(label, 12);
        for v in &vals {
            row.push_str(&pad(v, width));
        }
        row.push_str(if same { "  <-- 一致" } else { "  <-- 不同" });
        println!("{}", row);
    }

    if ok.len() == 2 {
        let (a, b) = (ok[0], ok[1]);
        println!();
        println!("差异摘要: {}  vs  {}", a.name, b.name);
        let fields: Vec<(&str, usize, usize)> = vec![
            ("0x07 闪存索引", 0x07, 1),
            ("0x0A NVM版本", 0x0A, 2),
            ("0x18 Vendor", 0x18, 2),
            ("0x1A DeviceID", 0x1A, 2),
            ("0x1C SubVendor", 0x1C, 2),
            ("0x1E SubDevice", 0x1E, 2),
            ("0x20 镜像类型", 0x20, 2),
            ("0x84 EEPID", 0x84, 4),
        ];
        for (label, off, size) in fields {
            if off + size > a.data.len() || off + size > b.data.len() {
                continue;
            }
            let va = &a.data[off..off + size];
            let vb = &b.data[off..off + size];
            if va != vb {
                let (sa, sb) = match size {
                    1 => (format!("0x{:02X}", va[0]), format!("0x{:02X}", vb[0])),
                    2 => (
                        format!("0x{:04X}", u16le_at(va, 0)),
                        format!("0x{:04X}", u16le_at(vb, 0)),
                    ),
                    _ => (
                        format!("0x{:08X}", u32le_at(va, 0)),
                        format!("0x{:08X}", u32le_at(vb, 0)),
                    ),
                };
                println!("   {} {} -> {}", pad(label, 14), sa, sb);
            }
        }
        if a.size == b.size {
            let n = a
                .data
                .iter()
                .zip(b.data.iter())
                .filter(|(x, y)| x != y)
                .count();
            println!(
                "   整片不同字节数: {} / {}  ({:.2}%)",
                n,
                a.size,
                100.0 * n as f64 / a.size as f64
            );
        } else {
            println!(
                "   两个文件大小不同（{} vs {}），跳过整片比对",
                a.size, b.size
            );
        }
    }
}

fn show_known() {
    println!("已记录的 EEPID / EtrackID 对照表（源码 top 区 known_eepid() 里维护）：");
    println!("{}", thin_line());
    for e in [
        0x800003FCu32,
        0x800002FC,
        0x800002F4,
        0x80000182,
        0x800003BB,
        0x800003BC,
    ] {
        println!("   0x{:08X}   {}", e, known_eepid(e));
    }
}

fn usage() {
    println!("{} {}", APP, VERSION);
    println!();
    println!("用法:");
    println!("  nvm_info.exe <镜像.bin> [镜像2.bin ...]    查看单个或多个镜像");
    println!("  nvm_info.exe <目录> [-r]                   扫描目录（加 -r 递归）");
    println!("  nvm_info.exe <包.zip> / <包.tar.gz>        不解压，直接读包内镜像");
    println!("  nvm_info.exe --list-known                  打印已记录的 EEPID 对照表");
    println!("  nvm_info.exe --json <镜像.bin>             机器可读输出");
    println!("  nvm_info.exe --help                        显示本帮助");
    println!();
    println!("输出字段:");
    println!("  MAC @0x00   闪存容量 @0x07   NVM 版本 @0x0A   Vendor @0x18");
    println!("  DeviceID @0x1A   Subsystem @0x1C   镜像类型 @0x20   EEPID/EtrackID @0x84");
    println!();
    println!("0x84 的依据：用 Intel 官方驱动包 Release_31.2.2 标定，其镜像文件名自带 EEPID 后缀，");
    println!("  实测 u32@0x84 一比一命中，且与 nvmupdate.cfg 的 EEPID: 字段三方互证。");
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn print_json(rs: &[ImageInfo]) {
    println!("[");
    for (i, r) in rs.iter().enumerate() {
        println!("  {{");
        println!("    \"file\": \"{}\",", json_escape(&r.name));
        println!("    \"source\": \"{}\",", json_escape(&r.source));
        println!("    \"size\": {},", r.size);
        println!("    \"md5\": \"{}\",", r.md5);
        println!("    \"ok\": {},", r.ok);
        if r.ok {
            println!("    \"mac\": \"{}\",", r.mac);
            println!("    \"flash_idx\": \"0x{:02X}\",", r.flash_idx);
            println!("    \"capacity\": \"{}\",", r.flash_label);
            println!("    \"imgtype\": \"0x{:04X}\",", r.imgtype);
            println!("    \"nvmver\": \"0x{:04X}\",", r.nvmver);
            println!("    \"nvmver_label\": \"{}\",", r.nvmver_label);
            println!("    \"vendor\": \"0x{:04X}\",", r.vendor);
            println!("    \"devid\": \"0x{:04X}\",", r.devid);
            println!("    \"devid_label\": \"{}\",", r.devid_label);
            println!(
                "    \"subsystem\": \"{:04X}:{:04X}\",",
                r.subvendor, r.subdevice
            );
            println!("    \"eepid\": \"0x{:08X}\",", r.eepid);
            println!("    \"eepid_note\": \"{}\",", json_escape(&r.eepid_note));
        }
        let notes: Vec<String> = r.notes.iter().map(|n| format!("\"{}\"", json_escape(n))).collect();
        println!("    \"notes\": [{}]", notes.join(", "));
        println!("  }}{}", if i + 1 == rs.len() { "" } else { "," });
    }
    println!("]");
}

// ============================================================================
// 8. main
// ============================================================================

fn main() {
    set_console_utf8();
    let argv: Vec<String> = env::args().skip(1).collect();

    let mut recursive = false;
    let mut json = false;
    let mut targets: Vec<String> = Vec::new();

    for a in &argv {
        match a.as_str() {
            "-r" | "--recursive" | "-R" => recursive = true,
            "--json" => json = true,
            "--list-known" => {
                show_known();
                return;
            }
            "-h" | "--help" | "/?" => {
                usage();
                return;
            }
            _ => targets.push(a.clone()),
        }
    }

    if targets.is_empty() {
        usage();
        println!();
        println!("提示：Windows 下可以直接把 .bin 文件拖到 nvm_info.exe 图标上运行。");
        process::exit(1);
    }

    let entries = collect(&targets, recursive);
    if entries.is_empty() {
        process::exit(1);
    }

    let results: Vec<ImageInfo> = entries
        .into_iter()
        .map(|e| parse_image(&e.name, &e.source, e.data))
        .collect();

    if json {
        print_json(&results);
    } else {
        for r in &results {
            show_result(r);
        }
        show_compare(&results);
    }

    let bad = results.iter().filter(|r| !r.ok).count();
    if bad == results.len() {
        process::exit(1);
    }
}
