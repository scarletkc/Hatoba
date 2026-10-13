export LC_ALL=C
system=$(uname -s 2>/dev/null)
if [ "$system" != Linux ] || [ ! -r /proc/stat ]; then
  echo "@unsupported $system"
  exit 0
fi
# The routing tables there are; awk stops at a file it cannot open.
routes=$(for f in /proc/net/route /proc/net/ipv6_route; do [ -r "$f" ] && printf '%s ' "$f"; done)
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
