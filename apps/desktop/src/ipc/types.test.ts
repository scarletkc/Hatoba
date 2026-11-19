import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { describe, expect, it } from "vitest";

const DIR = dirname(fileURLToPath(import.meta.url));

function bindingsEvents(): Map<string, string> {
  const path = resolve(DIR, "bindings.ts");
  const source = ts.createSourceFile(path, readFileSync(path, "utf8"), ts.ScriptTarget.Latest, true);
  const map = new Map<string, string>();
  const visit = (node: ts.Node) => {
    if (
      ts.isVariableDeclaration(node) &&
      ts.isIdentifier(node.name) &&
      node.name.text === "events" &&
      node.initializer &&
      ts.isObjectLiteralExpression(node.initializer)
    ) {
      for (const prop of node.initializer.properties) {
        if (!ts.isPropertyAssignment(prop) || !ts.isCallExpression(prop.initializer)) continue;
        const call = prop.initializer;
        const arg = call.arguments[0];
        const typeArg = call.typeArguments?.[0];
        if (arg && ts.isStringLiteral(arg) && typeArg) {
          map.set(arg.text, typeArg.getText(source));
        }
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return map;
}

function eventMapEntries(): Map<string, string> {
  const path = resolve(DIR, "types.ts");
  const source = ts.createSourceFile(path, readFileSync(path, "utf8"), ts.ScriptTarget.Latest, true);
  const map = new Map<string, string>();
  const visit = (node: ts.Node) => {
    if (ts.isInterfaceDeclaration(node) && node.name.text === "EventMap") {
      for (const member of node.members) {
        if (ts.isPropertySignature(member) && member.name && ts.isStringLiteral(member.name) && member.type) {
          map.set(member.name.text, member.type.getText(source));
        }
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return map;
}

describe("EventMap", () => {
  it("lists exactly the events in bindings.ts with matching payload types", () => {
    const expected = bindingsEvents();
    const actual = eventMapEntries();
    expect(expected.size).toBeGreaterThan(0);
    expect([...actual.keys()].sort()).toEqual([...expected.keys()].sort());
    for (const [name, payload] of expected) {
      expect(actual.get(name), name).toBe(payload);
    }
  });
});
