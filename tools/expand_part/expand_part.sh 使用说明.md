# expand.sh 使用说明

离线把 ImmortalWrt / OpenWrt **ext4-combined** 镜像的根分区（默认约 300MB）扩到整块盘。  
在 SystemRescue 这类 Live 环境里以 root 运行，目标盘必须**未挂载**。

---

## 一、什么时候用

- 安装好immortalwrt后
- 每次 `sysupgrade` 升级之后（升级会重写整盘镜像，分区退回 300MB，**要重跑一次**）

不适用：squashfs 镜像、非 ext2/3/4 文件系统、目标分区正在被当前系统使用。

---

## 二、怎么用

```bash
# 0) 从 SystemRescue U 盘启动，默认已是 root，不用输密码
# 1) 给脚本执行权限，假设脚本在 /root/expand_part.sh
chomd 755 /root/expand_part.sh

# 2) 先看看会做什么（推荐第一次都先跑这个）
./expand_part.sh --dry-run

# 3) 全交互式：选盘 → 选分区 → 选扩到多大 → 每步确认
./expand_part.sh

# 4) 可选-和3功能一样，已确认过环境，直接一键跑完
./expand_part.sh -d /dev/nvme0n1 -p 2 -y

# 5) 可选-和3功能一样，只扩到 60G，不吃满整盘
./expand_part.sh --size 60G
```

### 参数

| 参数              | 作用                               |
| --------------- | -------------------------------- |
| `-d, --disk`    | 指定磁盘，如 `/dev/nvme0n1`、`/dev/sda` |
| `-p, --part`    | 指定分区号，如 `2`                      |
| `-s, --size`    | 目标大小，见下节                         |
| `-y, --yes`     | 所有确认默认 yes，一键跑完                  |
| `-n, --dry-run` | 只打印将要执行的命令，不动盘                   |
| `--no-backup`   | 跳过分区表备份（不建议）                     |
| `-l, --log`     | 指定日志文件路径                         |
| `-h, --help`    | 查看帮助                             |

---

## 三、`--size` 怎么写

必须**带单位**，裸数字会被拒绝。

| 写法              | 实际大小       | 说明                  |
| --------------- | ---------- | ------------------- |
| `100%`          | 吃满整盘       | **默认值**             |
| `60G` / `60GiB` | 60 GiB     | 简写 `G` 按 1024 进制    |
| `60GB`          | 55.9 GiB   | 十进制，比 `60G` 小 4 GiB |
| `61440MiB`      | 60 GiB     | 等价写法                |
| `45%`           | 盘可用空间的 45% |                     |

**三条硬性限制**（脚本会拦，不让继续）：

1. 不能比当前分区小 —— 本脚本只扩不缩，缩容会丢数据
2. 不能超过盘尾 —— 注意 parted 按十进制 GB、`lsblk` 按 GiB 显示，别照抄数字
3. 不能和后面的分区重叠 —— 检测到会警告并给出不重叠的上限

扩完剩下的空间是空闲未分配，**以后可以再扩**（改回 `100%` 重跑即可）。

---

## 四、执行时会问什么

| 顺序     | 会问 / 会做                                 |
| ------ | --------------------------------------- |
| 1      | 列出所有磁盘，选一个（看容量和型号，别只看设备名）               |
| 2      | 列出分区，选根分区（默认高亮 ext4 那个，一般是 `p2`）        |
| 3      | 选扩到多大：`100%` / 不重叠上限 / 自定义              |
| 4      | 打印完整计划，**要你确认才动手**                      |
| Step 0 | 备份分区表（`sfdisk -d` + `sgdisk -b`）        |
| Step 1 | `sgdisk -e` 把 GPT 备份表挪回盘尾（MBR 盘自动跳过）    |
| Step 2 | `parted resizepart` 扩展分区 + `partprobe`  |
| Step 3 | `e2fsck -fy` 强制体检（不跑的话 resize2fs 会静默失败） |
| Step 4 | `resize2fs` 撑满文件系统                      |
| Step 5 | 用 `lsblk` + `dumpe2fs` 验证，打印扩容前后对比      |
| Step 6 | 提示 `reboot` + 拔 U 盘                     |

每个修改步骤都会先问 `[y/N]`，`-y` 可跳过。**起始扇区全程不动**，这是唯一真正危险的地方。

---

## 五、怎么判断成功

看 Step 5 的对比，三项都对上才算成：

```
  partition size    : 300.0 MB -> 120.0 GB
  filesystem blocks : 76800 -> 31449019 (block size 4096)
[OK] Filesystem grew by 119.7 GB.
```

然后 `reboot`（屏幕开始刷日志时拔 U 盘），回到 ImmortalWrt：

```bash
df -h /overlay     # 应接近硬盘实际大小，120G 的盘约 110G
```

---

## 六、常见问题

| 现象                                         | 原因               | 怎么办                                                              |
| ------------------------------------------ | ---------------- | ---------------------------------------------------------------- |
| `[WARN] e2fsck returned non-zero` 已修复为正常提示 | 退出码 1 = 已修正小错误   | **不是故障**，看 `FILE SYSTEM WAS MODIFIED` 即可；只有 `exit 4`（有错未修）脚本才会中止 |
| parted 报 `GPT PMBR size mismatch`          | GPT 备份表没挪到盘尾     | 脚本 Step 1 已处理；手动补 `sgdisk -e /dev/nvme0n1`                       |
| resize2fs 跑完大小没变                           | 没先跑 e2fsck，被静默拒绝 | 严格按 `e2fsck -fy` → `resize2fs` 顺序                                |
| `lsblk` 里看不到 nvme0n1                       | 固态没识别            | 换 U 口重插 / 重做启动盘 / 进 BIOS 确认 NVMe 没被关                             |
| U 盘启动不了                                    | Secure Boot 没关   | BIOS 里关掉，启动菜单选带 **UEFI** 的 U 盘                                   |
| 想还原分区表                                     | —                | `sfdisk /dev/nvme0n1 < partition-table-backup-xxx.sfdisk`        |

---

## 七、两个提醒

1. **备份文件要拷走**：`partition-table-backup-<盘名>-<时间戳>.sfdisk` / `.gpt`，别留在系统盘里，把两个都拷到 U 盘。
2. **别用 Windows 记事本另存**：会把行尾变成 CRLF，Linux 下报 `bad interpreter`。直接拖文件，或用 VS Code / Notepad++ 确认行尾是 **LF**。

# 八，实际示例参考

我跑过一次，输出如下，我是通过SSH连接到SystemRescue虚拟机测试的，脚本该名成expand.sh了：

```shell
[root@sysrescue ~]# ./expand.sh
Log file: ./expand-20260927-040027.log

==> Detected disks:
  1) /dev/nvme0n1      120G     nvme    VMware Virtual NVMe Disk
Select the disk holding ImmortalWrt [1-1]: 1
[OK] Target disk: /dev/nvme0n1 ( 120G, model: VMware Virtual NVMe Disk)
Partition table type: gpt

==> Partitions on /dev/nvme0n1:
nvme0n1      120G
nvme0n1p1     32M vfat 0fc63daf-8483-4772-8e79-3d69d8477de4
nvme0n1p2    300M ext4 0fc63daf-8483-4772-8e79-3d69d8477de4
nvme0n1p128  239K      21686148-6449-6e6f-744e-656564454649
Which partition number holds rootfs? [1 2 128] (default 2): 2
[OK] Target partition: /dev/nvme0n1p2
Filesystem: ext4
[OK] Safety checks passed: partition is unmounted and is not the running root.

==> Current layout
  disk            : /dev/nvme0n1 (120.0 GB)
  partition       : /dev/nvme0n1p2  #2
  start sector    : 66048   <- must stay unchanged
  end sector      : 680447   (partition size 300.0 MB)
  filesystem size : 300.0 MB

==> How far should the partition grow?
  1) 100%  - use all remaining space (default; parted may warn about overlap)
  3) custom - type a size yourself, e.g. 60G / 40960M
Choose [1]: 1
[OK] Target end: 100%

==> Plan
  disk        : /dev/nvme0n1
  partition   : /dev/nvme0n1p2 (#2), filesystem ext4
  grow to     : 100%
  steps       : backup partition table -> sgdisk -e -> parted resizepart
                -> partprobe -> e2fsck -fy -> resize2fs -> verify
  dry-run     : no
Proceed? (data on /dev/nvme0n1p2 will NOT be erased, but always keep a backup) [y/N] y

==> Step 0/6  Back up the partition table
[OK] Saved: ./partition-table-backup-nvme0n1-20260927-040038.sfdisk + ./partition-table-backup-nvme0n1-20260927-040038.gpt
Copy these files off the machine (to the USB stick) before rebooting.

==> Step 1/6  Move the GPT backup table to the end of the disk
The image is smaller than the disk, so the backup GPT still sits at the image's end.
Symptom if skipped: 'GPT PMBR size mismatch' and parted not seeing the full disk.
Run: sgdisk -e /dev/nvme0n1 [y/N] y
$ sgdisk -e /dev/nvme0n1
The operation has completed successfully.
$ parted -s /dev/nvme0n1 print
Model: VMware Virtual NVMe Disk (nvme)
Disk /dev/nvme0n1: 129GB
Sector size (logical/physical): 512B/512B
Partition Table: gpt
Disk Flags:

Number  Start   End     Size    File system  Name  Flags
128     17.4kB  262kB   245kB                      bios_grub
 1      262kB   33.8MB  33.6MB  fat16              legacy_boot
 2      33.8MB  348MB   315MB   ext4

[OK] GPT backup table relocated.

==> Step 2/6  Extend partition #2 to 100%
Run: parted -s /dev/nvme0n1 resizepart 2 100% [y/N] y
$ parted -s /dev/nvme0n1 resizepart 2 100%
$ partprobe /dev/nvme0n1
$ lsblk /dev/nvme0n1
NAME          MAJ:MIN RM SIZE RO TYPE MOUNTPOINTS
nvme0n1       259:0    0 120G  0 disk
├─nvme0n1p1   259:4    0  32M  0 part
├─nvme0n1p2   259:5    0 120G  0 part
└─nvme0n1p128 259:6    0 239K  0 part
[OK] Partition extended.

==> Step 3/6  Force a filesystem check (e2fsck -fy)
Skipping this makes resize2fs fail silently: no error, but no size change either.
Run: e2fsck -fy /dev/nvme0n1p2 [y/N] y
$ e2fsck -fy /dev/nvme0n1p2
e2fsck 1.47.4 (6-Mar-2025)
Pass 1: Checking inodes, blocks, and sizes
Pass 2: Checking directory structure
Pass 3: Checking directory connectivity
Pass 4: Checking reference counts
Pass 5: Checking group summary information
Padding at end of inode bitmap is not set. Fix? yes


rootfs: ***** FILE SYSTEM WAS MODIFIED *****
rootfs: 1557/19200 files (0.0% non-contiguous), 12005/76800 blocks
[WARN] e2fsck returned non-zero; review the output before continuing.

==> Step 4/6  Grow the filesystem (resize2fs)
Run: resize2fs /dev/nvme0n1p2 [y/N] y
$ resize2fs /dev/nvme0n1p2
resize2fs 1.47.4 (6-Mar-2025)
Resizing the filesystem on /dev/nvme0n1p2 to 31449019 (4k) blocks.
The filesystem on /dev/nvme0n1p2 is now 31449019 (4k) blocks long.

[OK] Filesystem grown.

==> Step 5/6  Verify
$ lsblk /dev/nvme0n1
NAME          MAJ:MIN RM SIZE RO TYPE MOUNTPOINTS
nvme0n1       259:0    0 120G  0 disk
├─nvme0n1p1   259:4    0  32M  0 part
├─nvme0n1p2   259:5    0 120G  0 part
└─nvme0n1p128 259:6    0 239K  0 part

  partition end sector : 680447 -> 251658206
  partition size       : 300.0 MB -> 120.0 GB
  filesystem blocks    : 76800 -> 31449019 (block size 4096)
  filesystem size      : 300.0 MB -> 120.0 GB
[OK] Filesystem grew by 119.7 GB.

==> Step 6/6  Next
  1. Run: reboot
  2. Pull the USB stick while the shutdown logs scroll (otherwise you boot the rescue system again).
  3. Back in ImmortalWrt: df -h /overlay   -> should be close to the disk size.

  Log of this run: ./expand-20260927-040027.log
  Partition-table backup: ./partition-table-backup-nvme0n1-20260927-040038.sfdisk ./partition-table-backup-nvme0n1-20260927-040038.gpt

Remember: every sysupgrade rewrites the whole image and resets the partition to its
original size. Just run this script again after upgrading.
[root@sysrescue ~]# reboot
client_loop: send disconnect: Connection reset
```

