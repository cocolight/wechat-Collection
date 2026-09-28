#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
nvm_info.py -- Intel Foxville (I225-V / I225-LM / I226-V) NVM 固件镜像离线体检工具

用途：不用上机、不用 eeupdate，直接在 PC 上读出任意 .bin 镜像的关键字段，
      尤其是 EtrackID (= EEPID, offset 0x84)。

用法：
    python nvm_info.py <镜像1> [镜像2 ...]      # 一个或多个镜像 / 目录
    python nvm_info.py <目录>                   # 扫描目录下所有 *.bin
    python nvm_info.py <目录> -r                # 递归扫描
    python nvm_info.py <包.zip> / <包.tar.gz>   # 直接读包内的 .bin（不用解压）
    python nvm_info.py --list-known             # 打印已记录的 EEPID 对照表
    python nvm_info.py <镜像> --json            # 机器可读输出

Windows 下不想敲命令：把 .bin 文件拖到 nvm_info.bat 上即可。

字段位置来源（已用 Intel 一手包标定，非推测）：
    Intel 官方驱动包 Release_31.2.2 -> NVMUpdatePackage/I225/I225_NVMUpdatePackage_v1_00_Linux/Linux_x64/
    其镜像文件名自带 EEPID 后缀，实测 u32@0x84 一比一命中：
        FoxPond1_I225_15F2_2MB_1p94_800003BB.bin -> @0x84 = 0x800003BB  [OK]
        Foxpond1_I225_15F2_LM_1MB_1p94_800003BC.bin -> @0x84 = 0x800003BC  [OK]
    且同目录 nvmupdate.cfg 的 "EEPID:" 字段同为这两个值 -> 文件名 / cfg / 镜像头三方互证。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import struct
import sys
import tarfile
import unicodedata
import zipfile

# ---------------------------------------------------------------- 已知值字典

# 闪存容量索引 @0x07
FLASH_IDX = {0x0D: "1MB", 0x05: "2MB"}
# 镜像类型 @0x20（与 0x07 互证）
IMAGE_TYPE = {0x8022: "1MB", 0x80A2: "2MB"}
# PCI Device ID @0x1A
DEVICE_ID = {
    0x15F3: "I225-V",
    0x15F2: "I225-LM",
    0x15F8: "I225-IT (未验证)",
    0x125C: "I226-V",
    0x125D: "I226-LM",
}
# 已实测过的 EEPID，随时往下补
KNOWN_EEPID = {
    0x800003FC: "Foxpond1_I225_15F3_V_1MB_1p94（15F3 / 1MB / NVM 1.94）",
    0x800002FC: "FXVL_15F3_V_1MB_1.89（15F3 / 1MB / NVM 1.89，Vendor 0x17AA）",
    0x800002F4: "FXVL_15F3_V_2MB_1.89（15F3 / 2MB / NVM 1.89）",
    0x80000182: "倍控 G31-1338 出厂备份（15F3 / 1MB / NVM 1.57）",
    0x800003BB: "Intel 官方 FoxPond1_I225_15F2_2MB_1p94",
    0x800003BC: "Intel 官方 Foxpond1_I225_15F2_LM_1MB_1p94",
}

IMG_SUFFIX = (".bin", ".img", ".eep", ".dat")


# ---------------------------------------------------------------- 工具函数

def setup_console() -> None:
    """Windows 控制台下强制 UTF-8 输出，避免中文/特殊符号炸 GBK 编码。"""
    if os.name == "nt":
        for s in (sys.stdout, sys.stderr):
            try:
                s.reconfigure(encoding="utf-8", errors="replace")
            except Exception:
                pass


def dw(s: str) -> int:
    """显示宽度：中文/全角字符按 2 列算。"""
    return sum(2 if unicodedata.east_asian_width(c) in ("W", "F") else 1 for c in s)


def pad(s: str, n: int) -> str:
    """按显示宽度补齐到 n 列（控制台等宽字体下对齐用）。"""
    s = str(s)
    return s + " " * max(0, n - dw(s))


def human_size(n: int) -> str:
    return {1048576: "1MB", 2097152: "2MB"}.get(n, "%.2f MB" % (n / 1048576.0))


def nvm_version_label(word: int) -> str:
    """
    把 NVM 版本字翻译成 Intel 的口语叫法。
    实测规律：版本 = 0x1000 + BCD(小版本)，例如 0x1094 -> 0x94 -> BCD 94 -> "1.94"。
    命中该规律返回 "1.94" 之类，否则返回 None。
    """
    hi, lo = (word >> 8) & 0xFF, word & 0xFF
    if hi == 0x10 and (lo >> 4) <= 9 and (lo & 0x0F) <= 9:
        return "%d.%02d" % (1, (lo >> 4) * 10 + (lo & 0x0F))
    return None


# ---------------------------------------------------------------- 核心解析

def parse_image(data: bytes, name: str, source: str = "") -> dict:
    v16 = lambda o: struct.unpack_from("<H", data, o)[0]
    v32 = lambda o: struct.unpack_from("<I", data, o)[0]

    r = {
        "file": name,
        "source": source or name,
        "size": len(data),
        "size_human": human_size(len(data)),
        "md5": hashlib.md5(data).hexdigest(),
        "ok": True,
        "notes": [],
    }

    if len(data) < 0x88:
        r["ok"] = False
        r["notes"].append("文件太小（不足 0x88 字节），不像 Foxville NVM 镜像")
        return r

    r["flash_idx"] = data[0x07]
    r["flash_idx_label"] = FLASH_IDX.get(data[0x07], "未知")
    r["imgtype"] = v16(0x20)
    r["imgtype_label"] = IMAGE_TYPE.get(r["imgtype"], "未知")

    w = v16(0x0A)
    r["nvmver"] = w
    lab = nvm_version_label(w)
    r["nvmver_label"] = lab if lab else "?"

    r["mac"] = ":".join("%02X" % b for b in data[0:6])
    r["vendor"] = v16(0x18)
    r["devid"] = v16(0x1A)
    r["devid_label"] = DEVICE_ID.get(r["devid"], "未知")
    r["subvendor"] = v16(0x1C)
    r["subdevice"] = v16(0x1E)
    r["eepid"] = v32(0x84)

    # ---- 一致性校验 ----
    if data[0x07] not in FLASH_IDX:
        r["notes"].append("0x07=0x%02X 不在已知容量索引表里，确认是不是 Foxville 镜像" % data[0x07])
    if r["imgtype"] not in IMAGE_TYPE:
        r["notes"].append("0x20=0x%04X 不是已知的镜像类型（1MB=0x8022 / 2MB=0x80A2）" % r["imgtype"])
    if FLASH_IDX.get(data[0x07]) and IMAGE_TYPE.get(r["imgtype"]) \
            and FLASH_IDX[data[0x07]] != IMAGE_TYPE[r["imgtype"]]:
        r["notes"].append("0x07 与 0x20 指向的容量不一致，头部可能损坏")
    if r["vendor"] not in (0x8086, 0x17AA, 0x1028, 0x8087):
        r["notes"].append("0x18 Vendor=0x%04X 非 0x8086，可能不是 Intel NVM 镜像" % r["vendor"])
    if r["devid"] not in DEVICE_ID:
        r["notes"].append("0x1A DeviceID=0x%04X 不是已知的 Foxville ID（15F3/15F2/125C/125D）" % r["devid"])

    # ---- 2MB dump 的真相：是不是同一份 1MB 被写了两遍 ----
    if len(data) == 2097152:
        if data[:0x100000] == data[0x100000:]:
            r["notes"].append(
                "前 1MB 与后 1MB 逐字节完全相同 -> 这是同一份 1MB 镜像被 dump 了两遍，"
                "不是四口各占一片；刷机请选 1MB 镜像")
        else:
            r["notes"].append("前 1MB 与后 1MB 内容不同，确为真实的 2MB 结构")

    if r["eepid"] in KNOWN_EEPID:
        r["eepid_note"] = KNOWN_EEPID[r["eepid"]]
    else:
        r["eepid_note"] = ""

    return r


# ---------------------------------------------------------------- 输入处理

def collect_targets(paths, recursive: bool) -> list:
    """把命令行参数展开成 [(显示名, 来源, bytes)] 列表。"""
    out = []
    for p in paths:
        p = os.path.abspath(p)
        if os.path.isdir(p):
            found = []
            if recursive:
                for root, _, files in os.walk(p):
                    for f in sorted(files):
                        if f.lower().endswith(IMG_SUFFIX):
                            found.append(os.path.join(root, f))
            else:
                found = [os.path.join(p, f) for f in sorted(os.listdir(p))
                         if f.lower().endswith(IMG_SUFFIX)]
            for f in found:
                out.append((os.path.basename(f), f, open(f, "rb").read()))
            if not found:
                print("[!] 目录里没有 %s 文件：%s" % ("/".join(IMG_SUFFIX), p))
        elif os.path.isfile(p):
            low = p.lower()
            if low.endswith(".zip"):
                with zipfile.ZipFile(p) as z:
                    for n in sorted(z.namelist()):
                        if n.lower().endswith(IMG_SUFFIX):
                            out.append((os.path.basename(n), p + "::" + n, z.read(n)))
            elif low.endswith((".tar.gz", ".tgz", ".tar")):
                with tarfile.open(p) as t:
                    for m in t.getmembers():
                        if m.isfile() and m.name.lower().endswith(IMG_SUFFIX):
                            out.append((os.path.basename(m.name), p + "::" + m.name,
                                        t.extractfile(m).read()))
            else:
                out.append((os.path.basename(p), p, open(p, "rb").read()))
        else:
            print("[!] 路径不存在：%s" % p)
    return out


# ---------------------------------------------------------------- 输出

def show_result(r: dict) -> None:
    print("=" * 72)
    print("文件    : %s" % r["file"])
    if r["source"] != r["file"]:
        print("来源    : %s" % r["source"])
    print("大小    : %d 字节 (%s)" % (r["size"], r["size_human"]))
    print("MD5     : %s" % r["md5"])
    if not r["ok"]:
        for n in r["notes"]:
            print("[x] %s" % n)
        return
    print("-" * 72)
    print("MAC 地址    @0x00 : %s" % r["mac"])
    print("闪存索引    @0x07 : 0x%02X  -> %s" % (r["flash_idx"], r["flash_idx_label"]))
    print("NVM 版本    @0x0A : 0x%04X  -> %s" % (r["nvmver"], r["nvmver_label"]))
    print("Vendor ID   @0x18 : 0x%04X" % r["vendor"])
    print("Device ID   @0x1A : 0x%04X  -> %s" % (r["devid"], r["devid_label"]))
    print("Subsystem   @0x1C : %04X:%04X" % (r["subvendor"], r["subdevice"]))
    print("镜像类型    @0x20 : 0x%04X  -> %s" % (r["imgtype"], r["imgtype_label"]))
    print("EEPID/Etrack@0x84 : 0x%08X" % r["eepid"])
    if r["eepid_note"]:
        print("             已知  : %s" % r["eepid_note"])
    if r["notes"]:
        print("-" * 72)
        for n in r["notes"]:
            print("[!] %s" % n)


def show_compare(results: list) -> None:
    ok = [r for r in results if r.get("ok")]
    if len(ok) < 2:
        return
    print()
    print("=" * 72)
    print("汇总对照")
    print("=" * 72)
    rows = [
        ("大小", lambda r: "%d (%s)" % (r["size"], r["size_human"])),
        ("MAC", lambda r: r["mac"]),
        ("闪存 0x07", lambda r: "0x%02X %s" % (r["flash_idx"], r["flash_idx_label"])),
        ("类型 0x20", lambda r: "0x%04X %s" % (r["imgtype"], r["imgtype_label"])),
        ("NVM 0x0A", lambda r: "0x%04X %s" % (r["nvmver"], r["nvmver_label"])),
        ("DevID 0x1A", lambda r: "0x%04X" % r["devid"]),
        ("EEPID 0x84", lambda r: "0x%08X" % r["eepid"]),
    ]
    heads = [r["file"][:26] for r in ok]
    width = max([14] + [dw(h) for h in heads]) + 2
    print(pad("", 12) + "".join(pad(h, width) for h in heads))
    for label, fn in rows:
        vals = [fn(r) for r in ok]
        same = len(set(vals)) == 1
        line = pad(label, 12) + "".join(pad(v, width) for v in vals)
        print(line + ("  <-- 一致" if same else "  <-- 不同"))

    if len(ok) == 2:
        a, b = ok
        print()
        print("差异摘要: %s  vs  %s" % (a["file"], b["file"]))
        pa, pb = a["_data"], b["_data"]
        for label, off, size in [
            ("0x07 闪存索引", 0x07, 1), ("0x0A NVM版本", 0x0A, 2),
            ("0x18 Vendor", 0x18, 2), ("0x1A DeviceID", 0x1A, 2),
            ("0x1C SubVendor", 0x1C, 2), ("0x1E SubDevice", 0x1E, 2),
            ("0x20 镜像类型", 0x20, 2), ("0x84 EEPID", 0x84, 4),
        ]:
            va, vb = pa[off:off + size], pb[off:off + size]
            if va != vb:
                if size == 1:
                    sa, sb = "0x%02X" % va[0], "0x%02X" % vb[0]
                elif size == 2:
                    sa, sb = ("0x%04X" % struct.unpack("<H", va), "0x%04X" % struct.unpack("<H", vb))
                else:
                    sa, sb = ("0x%08X" % struct.unpack("<I", va), "0x%08X" % struct.unpack("<I", vb))
                print("   %s %s -> %s" % (pad(label, 14), sa, sb))
        if a["size"] == b["size"]:
            n = sum(1 for i in range(a["size"]) if pa[i] != pb[i])
            print("   整片不同字节数: %d / %d  (%.2f%%)" % (n, a["size"], 100.0 * n / a["size"]))
        else:
            print("   两个文件大小不同（%d vs %d），跳过整片比对" % (a["size"], b["size"]))


def show_known() -> None:
    print("已记录的 EEPID / EtrackID 对照表（可在脚本顶部 KNOWN_EEPID 里补充）：")
    print("-" * 72)
    for k in sorted(KNOWN_EEPID):
        print("  0x%08X   %s" % (k, KNOWN_EEPID[k]))


# ---------------------------------------------------------------- main

def main() -> int:
    setup_console()
    ap = argparse.ArgumentParser(
        description="Intel I225/I226 (Foxville) NVM 固件镜像信息查看工具")
    ap.add_argument("targets", nargs="*", help="镜像文件 / 目录 / zip / tar.gz")
    ap.add_argument("-r", "--recursive", action="store_true", help="递归扫描目录")
    ap.add_argument("--json", action="store_true", help="输出 JSON")
    ap.add_argument("--list-known", action="store_true", help="打印已知 EEPID 表")
    args = ap.parse_args()

    if args.list_known:
        show_known()
        return 0
    if not args.targets:
        ap.print_help()
        print("\n提示：Windows 下可直接把 .bin 拖到 nvm_info.bat 上。")
        return 1

    items = collect_targets(args.targets, args.recursive)
    if not items:
        return 1

    results = []
    for name, src, data in items:
        r = parse_image(data, name, src)
        r["_data"] = data
        results.append(r)

    if args.json:
        for r in results:
            r.pop("_data", None)
        print(json.dumps(results, ensure_ascii=False, indent=2))
        return 0

    for r in results:
        show_result(r)
    show_compare(results)

    bad = [r for r in results if not r.get("ok")] 
    return 1 if bad and len(bad) == len(results) else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        pass
    except OSError as e:
        print("[x] 读取失败：%s" % e)
        sys.exit(2)
