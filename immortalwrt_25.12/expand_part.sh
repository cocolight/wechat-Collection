#!/usr/bin/env bash
#
# expand.sh —— 离线扩容交互式脚本
# 适用：ImmortalWrt / OpenWrt 的 ext4-combined 镜像写入硬盘后，根分区仍是几百 MB 的场景。
# 用法（在 SystemRescue 这类 Live 环境里以 root 运行，目标盘必须处于未挂载状态）：
#
#   ./expand.sh                       # 全交互式：选盘 → 选分区 → 逐步确认
#   ./expand.sh -d /dev/nvme0n1 -p 2  # 指定盘和分区号，仍逐步确认
#   ./expand.sh -d /dev/nvme0n1 -p 2 -y   # 一键跑完（确认项全部默认 yes）
#   ./expand.sh --dry-run             # 只打印将要执行的命令，不做任何改动
#   ./expand.sh --size 60G            # 只扩到 60GiB，不吃满整盘
#                                     # 可写：60G(=GiB) / 60GB(=十进制) / 61440MiB / 45% / 100%
#
# 设计原则：
#   1. 不硬编码设备名/分区号：磁盘、分区、目标大小都由探测 + 用户选择决定。
#   2. 每一步修改前都要确认；-y 可跳过，但破坏性操作仍会先打印摘要。
#   3. 前置安全闸门：目标分区必须未挂载、且不能是当前运行系统的根分区。
#   4. 动分区表前先备份；检测到目标分区后面还有分区（如 p128）时给出不重叠的安全上限。
#
# 注意：终端输出刻意用英文——SystemRescue 的 tty 字体通常不含中文字形，中文会显示成方块。

set -uo pipefail

# ------------------------------ 默认配置（可被命令行覆盖） ------------------------------
DISK=""          # 目标磁盘，如 /dev/nvme0n1、/dev/sda
PART_NUM=""      # 目标分区号，如 2
TARGET_SIZE=""   # 扩展到的大小：100% 或 60G 之类，留空则由脚本询问
DO_BACKUP=1      # 是否备份分区表
AUTO_YES=0       # -y：所有确认默认 yes
DRY_RUN=0        # --dry-run：只打印命令
LOG_FILE=""

# ------------------------------ 基础输出 ------------------------------
if [[ -t 1 && -z "${NO_COLOR:-}" ]]; then
    C_RED=$'\033[31m'; C_GRN=$'\033[32m'; C_YEL=$'\033[33m'
    C_BLU=$'\033[36m'; C_BLD=$'\033[1m'; C_OFF=$'\033[0m'
else
    C_RED=""; C_GRN=""; C_YEL=""; C_BLU=""; C_BLD=""; C_OFF=""
fi

info() { printf '%s\n' "$*"; }
step() { printf '\n%s%s\n' "${C_BLD}${C_BLU}==>${C_OFF} " "$*"; }
ok()   { printf '%s%s\n' "${C_GRN}[OK]${C_OFF} " "$*"; }
warn() { printf '%s%s\n' "${C_YEL}[WARN]${C_OFF} " "$*" >&2; }
die()  { printf '%s%s\n' "${C_RED}[FATAL]${C_OFF} " "$*" >&2; exit 1; }

log_line() { [[ -n "$LOG_FILE" ]] && printf '%s\n' "$*" >>"$LOG_FILE"; return 0; }

# 执行命令：输出同时显示并写入日志；dry-run 时只打印
run() {
    local rc
    if [[ $DRY_RUN -eq 1 ]]; then
        printf '%s[dry-run]%s %s\n' "$C_YEL" "$C_OFF" "$*"
        log_line "[dry-run] $*"
        return 0
    fi
    printf '%s$ %s%s\n' "$C_BLD" "$*" "$C_OFF"
    log_line "\$ $*"
    "$@" 2>&1 | tee -a "$LOG_FILE"
    return "${PIPESTATUS[0]}"
}

# 询问 yes/no；-y 时直接返回 0。$2 为默认值提示文案
confirm() {
    local q="$1" ans
    if [[ $AUTO_YES -eq 1 ]]; then
        printf '%s %s(auto: yes)%s\n' "$q" "$C_YEL" "$C_OFF"
        return 0
    fi
    while true; do
        read -r -p "$q [y/N] " ans
        case "${ans,,}" in
            y|yes) return 0 ;;
            n|no|"") return 1 ;;
            *) printf 'Please answer y or n.\n' ;;
        esac
    done
}

usage() {
    sed -n '3,12p' "$0" | sed 's/^# \{0,1\}//'
    exit 0
}

# ------------------------------ 参数解析 ------------------------------
while [[ $# -gt 0 ]]; do
    case "$1" in
        -d|--disk)     DISK="${2:?--disk needs a value}";       shift 2 ;;
        -p|--part)     PART_NUM="${2:?--part needs a value}";   shift 2 ;;
        -s|--size)     TARGET_SIZE="${2:?--size needs a value}"; shift 2 ;;
        -y|--yes)      AUTO_YES=1;      shift ;;
        -n|--dry-run)  DRY_RUN=1;       shift ;;
        --no-backup)   DO_BACKUP=0;     shift ;;
        -l|--log)      LOG_FILE="${2:?--log needs a value}";    shift 2 ;;
        -h|--help)     usage ;;
        *) die "Unknown option: $1 (try --help)" ;;
    esac
done

# ------------------------------ 小工具 ------------------------------
# 分区设备名规则：nvme/mmcblk/loop 需要 p 分隔，其余直接拼接
part_path() {
    local d="$1" n="$2"
    case "$d" in
        *nvme*|*mmcblk*|*loop*) printf '%s' "${d}p${n}" ;;
        *)                      printf '%s' "${d}${n}" ;;
    esac
}

human() { # 扇区数 -> 人类可读
    awk -v s="$1" 'BEGIN{
        v=s*512; split("B KB MB GB TB",u," "); i=1;
        while (v>=1024 && i<5) { v/=1024; i++ }
        printf (i==1 ? "%d %s" : "%.1f %s"), v, u[i]
    }'
}

# ------------------------------ 0. 前置检查 ------------------------------
[[ "$(id -u)" -eq 0 ]] || die "Must run as root. SystemRescue logs in as root by default."

MISSING=()
for t in lsblk blkid parted partprobe sgdisk e2fsck resize2fs dumpe2fs blockdev; do
    command -v "$t" >/dev/null 2>&1 || MISSING+=("$t")
done
if [[ ${#MISSING[@]} -gt 0 ]]; then
    warn "Missing tools: ${MISSING[*]}"
    [[ " ${MISSING[*]} " == *" e2fsck "* || " ${MISSING[*]} " == *" resize2fs "* ]] \
        && die "e2fsprogs is required for ext2/3/4. Install it first, or use a full rescue system."
    die "Install the missing tools first."
fi

LOG_FILE="${LOG_FILE:-./expand-$(date +%Y%m%d-%H%M%S).log}"
if [[ $DRY_RUN -eq 0 ]]; then
    : >"$LOG_FILE" 2>/dev/null || LOG_FILE="/tmp/expand-$(date +%Y%m%d-%H%M%S).log"
fi
info "Log file: ${LOG_FILE}"

# 当前运行系统的根设备，用于拒绝"在线扩容"
ROOT_SRC="$(findmnt -no SOURCE / 2>/dev/null || true)"

# 把 parted 的尺寸写法换算成"分区将占用的扇区数"，仅用于校验，不参与实际执行。
# 支持：s / B / kB / K / KiB / MB / M / MiB / GB / G / GiB / TB / T / TiB / <n>%
# 简写映射：G=GiB、M=MiB、T=TiB、K=KiB（分区语境习惯按 1024 进制）
# 不带单位一律拒绝——parted 的裸数字含义取决于当前 unit，太容易误解。
to_sectors() {
    local spec="$1" num unit bytes
    [[ "$spec" == *"%"* ]] && {
        num="${spec%\%}"
        [[ "$num" =~ ^[0-9]+(\.[0-9]+)?$ ]] || return 1
        awk -v n="$num" -v s="$CUR_START" -v e="$DISK_END" \
            'BEGIN{printf "%d", (e-s+1)*n/100}'
        return
    }
    [[ "$spec" =~ ^([0-9]+(\.[0-9]+)?)([A-Za-z]*)$ ]] || return 1
    num="${BASH_REMATCH[1]}"; unit="${BASH_REMATCH[3]}"
    case "$unit" in
        s|S)   awk -v n="$num" 'BEGIN{printf "%d", n}'; return ;;
        B)     bytes="$num" ;;
        kB)    bytes=$(awk -v n="$num" 'BEGIN{printf "%d", n*1000}') ;;
        K|KiB|kiB) bytes=$(awk -v n="$num" 'BEGIN{printf "%d", n*1024}') ;;
        MB)    bytes=$(awk -v n="$num" 'BEGIN{printf "%d", n*1000000}') ;;
        M|MiB) bytes=$(awk -v n="$num" 'BEGIN{printf "%d", n*1048576}') ;;
        GB)    bytes=$(awk -v n="$num" 'BEGIN{printf "%d", n*1000000000}') ;;
        G|GiB) bytes=$(awk -v n="$num" 'BEGIN{printf "%d", n*1073741824}') ;;
        TB)    bytes=$(awk -v n="$num" 'BEGIN{printf "%d", n*1000000000000}') ;;
        T|TiB) bytes=$(awk -v n="$num" 'BEGIN{printf "%d", n*1099511627776}') ;;
        *)     return 1 ;;
    esac
    awk -v b="$bytes" 'BEGIN{printf "%d", b/512}'
}

# ------------------------------ 1. 选择磁盘 ------------------------------
pick_disk() {
    local -a devs=()
    while read -r name; do
        [[ -n "$name" ]] && devs+=("/dev/$name")
    done < <(lsblk -dno NAME,TYPE 2>/dev/null | awk '$2=="disk"{print $1}')

    [[ ${#devs[@]} -gt 0 ]] || die "No block device found."

    step "Detected disks:"
    local i d
    for i in "${!devs[@]}"; do
        d="${devs[$i]}"
        printf '  %s) %-16s %-8s  %-6s  %s\n' \
            "$((i+1))" "$d" \
            "$(lsblk -dno SIZE "$d" 2>/dev/null)" \
            "$(lsblk -dno TRAN  "$d" 2>/dev/null || echo '?')" \
            "$(lsblk -dno MODEL "$d" 2>/dev/null || echo '')"
    done

    local choice=""
    while [[ -z "$choice" ]]; do
        read -r -p "Select the disk holding ImmortalWrt [1-${#devs[@]}]: " choice
        [[ "$choice" =~ ^[0-9]+$ ]] && (( choice>=1 && choice<=${#devs[@]} )) || { warn "Out of range."; choice=""; }
    done
    DISK="${devs[$((choice-1))]}"
}

if [[ -z "$DISK" ]]; then
    pick_disk
else
    [[ -b "$DISK" ]] || die "$DISK is not a block device."
fi
[[ -b "$DISK" ]] || die "$DISK is not a block device."
ok "Target disk: $DISK ($(lsblk -dno SIZE "$DISK"), model: $(lsblk -dno MODEL "$DISK" 2>/dev/null || echo '?'))"

# ------------------------------ 2. 选择分区 ------------------------------
PTTYPE="$(lsblk -dno PTTYPE "$DISK" 2>/dev/null || true)"
[[ -n "$PTTYPE" ]] || PTTYPE="$(blkid -p -o value -s PTTYPE "$DISK" 2>/dev/null || true)"
[[ -n "$PTTYPE" ]] || PTTYPE="$(parted -s "$DISK" print 2>/dev/null | awk -F': ' '/Partition Table/{print $2; exit}')"
info "Partition table type: ${PTTYPE:-unknown}"

pick_part() {
    step "Partitions on $DISK:"
    lsblk -ln -o NAME,SIZE,FSTYPE,PARTTYPE,MOUNTPOINTS "$DISK" 2>/dev/null || true

    local -a nums=()
    local pname ptype n base="${DISK##*/}"
    while read -r pname ptype; do
        [[ "$ptype" == "part" ]] || continue
        n="${pname#"$base"}"; n="${n#p}"
        [[ "$n" =~ ^[0-9]+$ ]] && nums+=("$n")
    done < <(lsblk -ln -o NAME,TYPE "$DISK" 2>/dev/null)
    [[ ${#nums[@]} -gt 0 ]] || die "No partition found on $DISK."

    local def=""
    local n
    for n in "${nums[@]}"; do
        local fst
        fst="$(blkid -o value -s TYPE "$(part_path "$DISK" "$n")" 2>/dev/null || true)"
        [[ "$fst" == ext4 ]] && { def="$n"; break; }
    done

    while [[ -z "$PART_NUM" ]]; do
        read -r -p "Which partition number holds rootfs? [${nums[*]}]${def:+ (default $def)}: " PART_NUM
        [[ -z "$PART_NUM" ]] && PART_NUM="$def"
        [[ " ${nums[*]} " == *" $PART_NUM "* ]] || { warn "No such partition on $DISK."; PART_NUM=""; }
    done
}

if [[ -z "$PART_NUM" ]]; then
    pick_part
else
    [[ -b "$(part_path "$DISK" "$PART_NUM")" ]] || die "Partition $PART_NUM does not exist on $DISK."
fi

PART_DEV="$(part_path "$DISK" "$PART_NUM")"
[[ -b "$PART_DEV" ]] || die "$PART_DEV is not a block device."
ok "Target partition: $PART_DEV"

# 文件系统类型检查（本脚本走 e2fsprogs，只覆盖 ext2/3/4）
FSTYPE="$(blkid -o value -s TYPE "$PART_DEV" 2>/dev/null || true)"
[[ -n "$FSTYPE" ]] || FSTYPE="$(lsblk -no FSTYPE "$PART_DEV" 2>/dev/null || true)"
info "Filesystem: ${FSTYPE:-unknown}"
case "$FSTYPE" in
    ext2|ext3|ext4) ;;
    *) die "$PART_DEV is ${FSTYPE:-unknown}, not ext2/3/4. This script only grows ext filesystems." ;;
esac

# ------------------------------ 3. 安全闸门 ------------------------------
mp="$(lsblk -no MOUNTPOINTS "$PART_DEV" 2>/dev/null || lsblk -no MOUNTPOINT "$PART_DEV" 2>/dev/null || true)"
if [[ -n "${mp// /}" ]]; then
    die "$PART_DEV is mounted on: $mp
  Offline resize requires it unmounted. Boot SystemRescue from USB and run again."
fi
if [[ -n "$ROOT_SRC" && "$ROOT_SRC" == *"${PART_DEV##*/}"* ]]; then
    die "$PART_DEV looks like the root filesystem of the running system.
  Refusing to resize an online root filesystem. Reboot from the rescue USB."
fi
ok "Safety checks passed: partition is unmounted and is not the running root."

# ------------------------------ 4. 采集当前布局 ------------------------------
read_layout() {
    CUR_START="$(parted -s "$DISK" unit s print 2>/dev/null \
        | awk -v n="$PART_NUM" '$1==n {gsub(/s/,"",$2); gsub(/s/,"",$3); print $2; exit}')"
    CUR_END="$(parted -s "$DISK" unit s print 2>/dev/null \
        | awk -v n="$PART_NUM" '$1==n {gsub(/s/,"",$2); gsub(/s/,"",$3); print $3; exit}')"
    DISK_END="$(parted -s "$DISK" unit s print 2>/dev/null \
        | awk '/^Disk /{for(i=1;i<=NF;i++) if($i ~ /^[0-9]+s$/){gsub(/s/,"",$i); print $i; exit}}')"
    NEXT_START="$(parted -s "$DISK" unit s print 2>/dev/null \
        | awk -v s="$CUR_START" 'NF>=3 && $2 ~ /^[0-9]+s$/ {gsub(/s/,"",$2); v=$2+0;
             if (v > s+0 && (m=="" || v<m)) m=v} END{if(m!="") print m}')"
}
read_layout
[[ -n "$CUR_START" && -n "$CUR_END" ]] || die "Cannot parse the partition layout of $DISK."

FS_BLOCKS_BEFORE="$(dumpe2fs -h "$PART_DEV" 2>/dev/null | awk -F: '/^Block count/{gsub(/ /,"",$2); print $2}')"
FS_BSIZE="$(dumpe2fs -h "$PART_DEV" 2>/dev/null | awk -F: '/^Block size/{gsub(/ /,"",$2); print $2}')"

step "Current layout"
printf '  disk            : %s (%s)\n' "$DISK" "$(human "$DISK_END")"
printf '  partition       : %s  #%s\n' "$PART_DEV" "$PART_NUM"
printf '  start sector    : %s   <- must stay unchanged\n' "$CUR_START"
printf '  end sector      : %s   (partition size %s)\n' "$CUR_END" "$(human "$((CUR_END-CUR_START+1))")"
printf '  filesystem size : %s\n' "$(human "$((FS_BLOCKS_BEFORE*FS_BSIZE/512))")"
if [[ -n "$NEXT_START" ]]; then
    warn "Partition(s) exist after #$PART_NUM (next start sector $NEXT_START)."
    warn "Growing to 100% may overlap them (e.g. the small p128 reserved partition)."
fi

# ------------------------------ 5. 目标大小 ------------------------------
if [[ -z "$TARGET_SIZE" ]]; then
    step "How far should the partition grow?"
    printf '  1) 100%%  - use all remaining space (default; parted may warn about overlap)\n'
    if [[ -n "$NEXT_START" ]]; then
        local_safe_mb=$(( (NEXT_START*512 - 1048576) / 1048576 ))
        printf '  2) %sMiB - stop right before the next partition (no overlap)\n' "$local_safe_mb"
    fi
    printf '  3) custom - type a size yourself, e.g. 60G / 60GiB / 61440MiB\n'
    read -r -p "Choose [1]: " size_choice
    case "${size_choice:-1}" in
        1|"")  TARGET_SIZE="100%" ;;
        2)     [[ -n "$NEXT_START" ]] && TARGET_SIZE="${local_safe_mb}MiB" || { warn "No next partition; using 100%."; TARGET_SIZE="100%"; } ;;
        3)     read -r -p "Size (e.g. 60G = 60GiB, 60GB = decimal, 100%): " TARGET_SIZE ;;
        *)     TARGET_SIZE="100%" ;;
    esac
fi
[[ -n "$TARGET_SIZE" ]] || TARGET_SIZE="100%"

# 目标大小校验：不能比现在小（本脚本只扩不缩），不能超出盘尾，重叠要警告
CUR_SECTORS=$(( CUR_END - CUR_START + 1 ))
WANT_SECTORS="$(to_sectors "$TARGET_SIZE")" || die \
    "Cannot parse size '$TARGET_SIZE'. Use an explicit unit: 60G / 60GiB / 60GB / 61440MiB / 100%"
[[ -n "$WANT_SECTORS" && "$WANT_SECTORS" -gt 0 ]] || die "Size '$TARGET_SIZE' resolves to 0 sectors."

WANT_END=$(( CUR_START + WANT_SECTORS - 1 ))
CUR_GIB="$(awk -v s="$CUR_SECTORS" 'BEGIN{printf "%.2f GiB", s*512/1073741824}')"
WANT_GIB="$(awk -v s="$WANT_SECTORS" 'BEGIN{printf "%.2f GiB", s*512/1073741824}')"

step "Target size check"
printf '  requested        : %s\n' "$TARGET_SIZE"
printf '  partition size   : %s sectors (%s)  ->  %s sectors (%s)\n' \
    "$CUR_SECTORS" "$CUR_GIB" "$WANT_SECTORS" "$WANT_GIB"
printf '  new end sector   : %s (disk ends at %s)\n' "$WANT_END" "$DISK_END"

if (( WANT_SECTORS <= CUR_SECTORS )); then
    die "Target ($WANT_GIB) is NOT larger than the current partition ($CUR_GIB).
  This script only GROWS partitions. Shrinking would destroy data at the tail.
  If you really need to shrink: back up first, shrink the FILESYSTEM (resize2fs)
  before the PARTITION, and never the other way around."
fi
if (( WANT_END > DISK_END )); then
    die "Target end sector $WANT_END exceeds the disk end ($DISK_END). Pick something smaller."
fi
if [[ -n "$NEXT_START" ]] && (( WANT_END >= NEXT_START )); then
    warn "Target overlaps the partition starting at sector $NEXT_START."
    if [[ "$TARGET_SIZE" != "100%" ]]; then
        local_safe_mb=$(( (NEXT_START*512 - 1048576) / 1048576 ))
        if confirm "Use the non-overlapping size ${local_safe_mb}MiB instead?"; then
            TARGET_SIZE="${local_safe_mb}MiB"
            WANT_SECTORS="$(to_sectors "$TARGET_SIZE")"
            WANT_END=$(( CUR_START + WANT_SECTORS - 1 ))
            ok "Adjusted to $TARGET_SIZE (end sector $WANT_END)."
        fi
    fi
fi
ok "Target: $TARGET_SIZE (~$WANT_GIB, end sector $WANT_END)"

# ------------------------------ 6. 最终确认 ------------------------------
step "Plan"
cat <<EOF
  disk        : $DISK
  partition   : $PART_DEV (#$PART_NUM), filesystem $FSTYPE
  grow to     : $TARGET_SIZE
  steps       : backup partition table -> sgdisk -e -> parted resizepart
                -> partprobe -> e2fsck -fy -> resize2fs -> verify
  dry-run     : $([[ $DRY_RUN -eq 1 ]] && echo yes || echo no)
EOF
if ! confirm "Proceed? (data on $PART_DEV will NOT be erased, but always keep a backup)"; then
    info "Aborted by user. Nothing was changed."
    exit 0
fi

# ------------------------------ 7. 备份分区表 ------------------------------
if [[ $DO_BACKUP -eq 1 ]]; then
    step "Step 0/6  Back up the partition table"
    BK="./partition-table-backup-${DISK##*/}-$(date +%Y%m%d-%H%M%S)"
    if [[ $DRY_RUN -eq 0 ]]; then
        sfdisk -d "$DISK" > "${BK}.sfdisk" 2>/dev/null || warn "sfdisk dump failed."
        if [[ "$PTTYPE" == "gpt" ]]; then
            sgdisk -b "${BK}.gpt" "$DISK" >/dev/null 2>&1 || warn "sgdisk backup failed."
        fi
        ok "Saved: ${BK}.sfdisk $( [[ -f "${BK}.gpt" ]] && echo "+ ${BK}.gpt" )"
        info "Copy these files off the machine (to the USB stick) before rebooting."
    else
        run sfdisk -d "$DISK"
    fi
fi

# ------------------------------ 8. 修 GPT 备份表 ------------------------------
if [[ "$PTTYPE" == "gpt" ]]; then
    step "Step 1/6  Move the GPT backup table to the end of the disk"
    info "The image is smaller than the disk, so the backup GPT still sits at the image's end."
    info "Symptom if skipped: 'GPT PMBR size mismatch' and parted not seeing the full disk."
    if confirm "Run: sgdisk -e $DISK"; then
        run sgdisk -e "$DISK" || die "sgdisk -e failed. Check the output above."
        run parted -s "$DISK" print
        ok "GPT backup table relocated."
    else
        warn "Skipped. If parted later reports 'GPT PMBR size mismatch', run: sgdisk -e $DISK"
    fi
else
    step "Step 1/6  Partition table is '$PTTYPE', skipping GPT relocation"
fi

# ------------------------------ 9. 扩展分区 ------------------------------
step "Step 2/6  Extend partition #$PART_NUM to $TARGET_SIZE"
if confirm "Run: parted -s $DISK resizepart $PART_NUM $TARGET_SIZE"; then
    if ! run parted -s "$DISK" resizepart "$PART_NUM" "$TARGET_SIZE"; then
        if [[ -n "$NEXT_START" ]]; then
            local_safe_mb=$(( (NEXT_START*512 - 1048576) / 1048576 ))
            warn "parted refused. Likely an overlap with the partition starting at sector $NEXT_START."
            if confirm "Retry with the non-overlapping size ${local_safe_mb}MiB?"; then
                run parted -s "$DISK" resizepart "$PART_NUM" "${local_safe_mb}MiB" \
                    || die "Still failed. Restore with: sfdisk $DISK < ${BK:-backup}.sfdisk"
            else
                die "Aborted at the partition step. Layout unchanged."
            fi
        else
            die "parted resizepart failed. Layout may be unchanged; see the log."
        fi
    fi
    run partprobe "$DISK"
    udevadm settle 2>/dev/null || true
    run lsblk "$DISK"
    ok "Partition extended."
else
    die "Aborted by user before touching the partition table."
fi

# ------------------------------ 10. 文件系统体检 ------------------------------
step "Step 3/6  Force a filesystem check (e2fsck -fy)"
info "Skipping this makes resize2fs fail silently: no error, but no size change either."
if confirm "Run: e2fsck -fy $PART_DEV"; then
    run e2fsck -fy "$PART_DEV"
    fsck_rc=$?
    # e2fsck exit codes:
    #   0 = clean   1 = errors corrected   2 = corrected, reboot advised
    #   4 = errors left uncorrected   8 = operational error
    #  16 = usage error  32 = cancelled  128 = shared-library error
    case "$fsck_rc" in
        0)   if [[ $DRY_RUN -eq 1 ]]; then info "(dry-run) result check skipped.";
             else ok "Filesystem is clean, nothing to fix."; fi ;;
        1|2) ok "e2fsck repaired minor inconsistencies (exit $fsck_rc). This is NORMAL, not a failure." ;;
        4)   die "e2fsck left errors uncorrected (exit 4). Do NOT run resize2fs. Fix manually first." ;;
        8)   die "e2fsck hit an operational error (exit 8). Aborting." ;;
        32)  die "e2fsck was cancelled (exit 32). Aborting." ;;
        *)   die "e2fsck exited with $fsck_rc. Aborting; check the log: $LOG_FILE" ;;
    esac
else
    die "Aborted. e2fsck is mandatory before resize2fs."
fi

# ------------------------------ 11. 撑满文件系统 ------------------------------
step "Step 4/6  Grow the filesystem (resize2fs)"
if confirm "Run: resize2fs $PART_DEV"; then
    run resize2fs "$PART_DEV" || die "resize2fs failed. Nothing was lost; the filesystem is unchanged."
    ok "Filesystem grown."
else
    die "Aborted. The filesystem still has its old size."
fi

# ------------------------------ 12. 验证 ------------------------------
step "Step 5/6  Verify"
OLD_END="$CUR_END"
read_layout
FS_BLOCKS_AFTER="$(dumpe2fs -h "$PART_DEV" 2>/dev/null | awk -F: '/^Block count/{gsub(/ /,"",$2); print $2}')"
run lsblk "$DISK"
printf '\n'
printf '  partition end sector : %s -> %s\n' "$OLD_END" "$CUR_END"
printf '  partition size       : %s -> %s\n' \
    "$(human "$((OLD_END-CUR_START+1))")" "$(human "$((CUR_END-CUR_START+1))")"
printf '  filesystem blocks    : %s -> %s (block size %s)\n' \
    "$FS_BLOCKS_BEFORE" "$FS_BLOCKS_AFTER" "$FS_BSIZE"
printf '  filesystem size      : %s -> %s\n' \
    "$(human "$((FS_BLOCKS_BEFORE*FS_BSIZE/512))")" "$(human "$((FS_BLOCKS_AFTER*FS_BSIZE/512))")"

if [[ -n "$FS_BLOCKS_BEFORE" && -n "$FS_BLOCKS_AFTER" && "$FS_BLOCKS_AFTER" -gt "$FS_BLOCKS_BEFORE" ]]; then
    ok "Filesystem grew by $(human "$(( (FS_BLOCKS_AFTER-FS_BLOCKS_BEFORE)*FS_BSIZE/512 ))")."
else
    warn "Block count did not increase. Check: was e2fsck run first? Is the disk really bigger?"
fi

# ------------------------------ 13. 收尾 ------------------------------
step "Step 6/6  Next"
cat <<EOF
  1. Run: reboot
  2. Pull the USB stick while the shutdown logs scroll (otherwise you boot the rescue system again).
  3. Back in ImmortalWrt: df -h /overlay   -> should be close to the disk size.

  Log of this run: $LOG_FILE
EOF
if [[ -n "${BK:-}" ]]; then
    printf '  Partition-table backup: %s.sfdisk %s\n' "$BK" "$( [[ -f "${BK}.gpt" ]] && echo "${BK}.gpt" )"
fi
printf '\nRemember: every sysupgrade rewrites the whole image and resets the partition to its\n'
printf 'original size. Just run this script again after upgrading.\n'
