# What command suggestions take from the server (TERM-16), run as `sh -s`: the login shell's name,
# the last lines of the bash, zsh and fish histories, and the executables on PATH. Each part follows
# a line of its own that starts with `hatoba-suggest`, so anything a startup file prints comes
# before the first one, and a file without a line break at its end does not run into the next one.
printf 'hatoba-suggest shell %s\n' "${SHELL##*/}"
echo 'hatoba-suggest bash'
[ -f "$HOME/.bash_history" ] && tail -n 2000 "$HOME/.bash_history" 2>/dev/null
echo
echo 'hatoba-suggest zsh'
[ -f "$HOME/.zsh_history" ] && tail -n 2000 "$HOME/.zsh_history" 2>/dev/null
echo
echo 'hatoba-suggest fish'
f=${XDG_DATA_HOME:-$HOME/.local/share}/fish/fish_history
[ -f "$f" ] && tail -n 8000 "$f" 2>/dev/null
echo
echo 'hatoba-suggest path'
set -f
IFS=:
set -- $PATH
unset IFS
set +f
for d do
  case $d in /*) ;; *) continue ;; esac
  [ -d "$d" ] || continue
  for f in "$d"/*; do
    [ -f "$f" ] && [ -x "$f" ] && printf '%s\n' "${f##*/}"
  done
done
echo 'hatoba-suggest end'
