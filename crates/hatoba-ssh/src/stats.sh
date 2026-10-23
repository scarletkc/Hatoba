export LC_ALL=C
system=$(uname -s 2>/dev/null)
if [ "$system" != Darwin ] && { [ "$system" != Linux ] || [ ! -r /proc/stat ]; }; then
  echo "@unsupported $system"
  exit 0
fi
# The dashboard (HOST-14) also asks for every filesystem ("$2" is "mounts"). A network
# filesystem that stopped answering would hold `df` up, so `timeout` bounds it where present
# (BusyBox before 1.30 takes the seconds as -t).
dft=
if [ "$2" = mounts ]; then
  if timeout 5 true 2>/dev/null; then dft="timeout 5"
  elif timeout -t 5 true 2>/dev/null; then dft="timeout -t 5"
  fi
fi
n=0

# macOS has no /proc: its own tools give the same readings, printed as the Linux files read.
if [ "$system" = Darwin ]; then
  # A stock Mac has no `timeout`; perl's alarm, which outlives exec, bounds `df` there.
  if [ "$2" = mounts ] && [ -z "$dft" ] && perl -e 1 2>/dev/null; then
    dft="perl -e alarm(5);exec(@ARGV)"
  fi
  echo "@cpus $(sysctl -n hw.ncpu)"
  # iostat prints the CPU shares and the load averages each interval, the first line since
  # boot. Added up, the shares stand in for the tick counters of /proc/stat.
  user=0 sys=0 idle=0
  iostat -n 0 -w "$1" | while read -r us sy id l1 l5 l15; do
    case $us$sy$id in '' | *[!0-9]*) continue ;; esac
    user=$((user + us)) sys=$((sys + sy)) idle=$((idle + id))
    echo @cpu
    echo "cpu $user 0 $sys $idle"
    echo @mem
    # In use as Activity Monitor counts it: app memory, wired and compressed.
    { sysctl -n hw.memsize vm.swapusage; vm_stat; } 2>/dev/null | awk '
      NR == 1 { total = $1 / 1024; next }
      $1 == "total" { swap = $3 * 1024; swap_free = $9 * 1024; next }
      /page size of/ { for (i = 1; i < NF; i++) if ($i == "of") page = $(i + 1) / 1024 }
      /^Anonymous pages:|^Pages wired down:|^Pages occupied by compressor:/ { used += $NF }
      /^Pages purgeable:/ { used -= $NF }
      END {
        available = total - used * page
        if (available < 0) available = 0
        printf "MemTotal: %d kB\nMemAvailable: %d kB\n", total, available
        printf "SwapTotal: %d kB\nSwapFree: %d kB\n", swap, swap_free
      }
    '
    echo @net
    # Traffic counts on the interfaces of the default routes, or else on all that are up but
    # loopback, in the columns of /proc/net/dev.
    { route -n get default; route -n get -inet6 default; echo @; netstat -ibn; } 2>/dev/null | awk '
      $0 == "@" { listed = 1; next }
      !listed { if ($1 == "interface:") want[$2] = 1; next }
      $3 ~ /^<Link#/ {
        name[++count] = $1; down[count] = sub(/\*$/, "", name[count])
        rx[count] = $(NF - 4); tx[count] = $(NF - 1)
      }
      END {
        for (k in want) any = 1
        for (i = 1; i <= count; i++) {
          if (any ? (name[i] in want) : (!down[i] && name[i] !~ /^lo[0-9]/)) {
            print name[i] ": " rx[i] " 0 0 0 0 0 0 0 " tx[i]
          }
        }
      }
    '
    echo @load
    echo "$l1 $l5 $l15"
    echo @uptime
    # To the microsecond where the kernel gives the time that precisely, else to the second.
    { sysctl -n kern.boottime kern.monotonicclock_usecs; date +%s; } 2>/dev/null | awk '
      /sec =/ { boot = $4 + $7 / 1000000 }
      NF == 2 && $1 > 1e12 { now = $1 / 1000000 }
      NF == 1 && !now { now = $1 }
      END { printf "%.2f\n", now - boot }
    '
    # APFS volumes share their container's space, so a volume's use is its size less what is
    # available, the container's use as Finder shows it.
    echo @disk
    df -kP / 2>/dev/null | awk 'NR > 1 && NF >= 6 { $3 = $2 - $4 } 1'
    # Every filesystem Finder shows, and /. The ones it hides (nobrowse), such as the system's
    # other volumes and /dev, and read-only ones are left out. Only APFS volumes take their
    # container's use; other filesystems keep what df counts as used.
    if [ "$2" = mounts ] && [ $((n % 5)) -eq 0 ]; then
      echo @mounts
      { mount; echo @; $dft df -kP; } 2>/dev/null | awk '
        $0 == "@" { listed = 1; next }
        !listed {
          rest = substr($0, index($0, " on ") + 4)
          for (j = length(rest) - 1; j > 1; j--) if (substr(rest, j, 2) == " (") break
          m = substr(rest, 1, j - 1); opts = substr(rest, j + 2, length(rest) - j - 2) ", "
          type[m] = substr(opts, 1, index(opts, ",") - 1)
          hidden[m] = opts ~ /, (nobrowse|read-only), /
          next
        }
        {
          for (i = 5; i < NF; i++) if ($i ~ /%$/) break
          if (i >= NF) next
          m = $(i + 1); for (k = i + 2; k <= NF; k++) m = m " " $k
          if (m != "/" && hidden[m]) next
          if (type[m] == "apfs") $(i - 2) = $(i - 3) - $(i - 1)
          print ((m in type) ? type[m] : "-"), $0
        }
      '
    fi
    n=$((n + 1))
    echo @end
  done
  exit
fi

# The routing tables there are; awk stops at a file it cannot open.
routes=$(for f in /proc/net/route /proc/net/ipv6_route; do [ -r "$f" ] && printf '%s ' "$f"; done)
echo "@cpus $(grep -c '^cpu[0-9]' /proc/stat)"
while :; do
  echo @cpu
  read -r line < /proc/stat && echo "$line"
  echo @mem
  grep -E '^(MemTotal|MemFree|MemAvailable|Buffers|Cached|SwapTotal|SwapFree):' /proc/meminfo
  echo @net
  # Traffic counts on the interfaces of the current default routes, or else on all but loopback.
  awk '
    FILENAME == "/proc/net/route" { if ($2 == "00000000") want[$1] = 1; next }
    FILENAME == "/proc/net/ipv6_route" { if ($1 ~ /^0+$/ && $2 == "00" && $10 != "lo") want[$10] = 1; next }
    FNR > 2 { n = $0; sub(/:.*/, "", n); gsub(/[ \t]/, "", n); dev[++count] = $0; name[count] = n }
    END {
      for (k in want) any = 1
      for (i = 1; i <= count; i++) if (any ? (name[i] in want) : name[i] != "lo") print dev[i]
    }
  ' $routes /proc/net/dev
  echo @load
  read -r line < /proc/loadavg && echo "$line"
  echo @uptime
  read -r line < /proc/uptime && echo "$line"
  echo @disk
  df -kP / 2>/dev/null
  # Every filesystem in each fifth block, each line after its type from the mount table, where
  # an escaped space in a mount point reads \040.
  if [ "$2" = mounts ] && [ $((n % 5)) -eq 0 ]; then
    echo @mounts
    $dft df -kP 2>/dev/null | awk -v table=/proc/self/mounts '
      BEGIN {
        while ((getline entry < table) > 0) {
          split(entry, f, " "); m = f[2]; gsub(/\\040/, " ", m); gsub(/\\011/, "\t", m); type[m] = f[3]
        }
      }
      {
        for (i = 5; i < NF; i++) if ($i ~ /%$/) break
        m = $(i + 1); for (j = i + 2; j <= NF; j++) m = m " " $j
        print ((m in type) ? type[m] : "-"), $0
      }
    '
  fi
  n=$((n + 1))
  echo @end
  sleep "$1" || exit
done
