/**
 * §13.5: which `run_command` command lines run without asking in manual approval mode. A command line
 * runs without asking when every command in it only reads (`ls`, `cat`, `grep`, `git log`, …) or starts
 * with a prefix the user allowed (AI-19). Whatever this cannot tell for sure asks.
 */

export { dialectOf, HIDDEN, splitCommands, type ShellDialect } from "./commandRules/split";
export {
  allowancePrefixes,
  commandAllowed,
  readsOnlyCommand,
  savablePrefixes,
  windowsName,
} from "./commandRules/prefixes";
