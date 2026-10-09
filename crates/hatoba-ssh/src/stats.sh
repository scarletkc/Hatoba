export LC_ALL=C
system=$(uname -s 2>/dev/null)
if [ "$system" != Linux ] || [ ! -r /proc/stat ]; then
  echo "@unsupported $system"
  exit 0
fi
# Traffic counts on the interfaces of the default routes, or else on all but loopback.
ifaces=$({
  awk '$2 == "00000000" { print $1 }' /proc/net/route
  awk '$1 ~ /^0+$/ && $2 == "00" && $10 != "lo" { print $10 }' /proc/net/ipv6_route
} 2>/dev/null | sort -u | tr '\n' ' ')
echo "@cpus $(grep -c '^cpu[0-9]' /proc/stat)"
while :; do
  echo @cpu
  read -r line < /proc/stat && echo "$line"
  echo @mem
  grep -E '^(MemTotal|MemFree|MemAvailable|Buffers|Cached|SwapTotal|SwapFree):' /proc/meminfo
  echo @net
  awk -F: -v want=" $ifaces " 'NR > 2 { n = $1; gsub(/[ \t]/, "", n); if (want == "  " ? n != "lo" : index(want, " " n " ")) print }' /proc/net/dev
  echo @load
  read -r line < /proc/loadavg && echo "$line"
  echo @uptime
  read -r line < /proc/uptime && echo "$line"
  echo @disk
  df -kP / 2>/dev/null
  echo @end
  sleep "$1" || exit
done
