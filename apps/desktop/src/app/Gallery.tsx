import { useEffect, useState, type ReactNode } from "react";
import {
  Badge,
  Button,
  Checkbox,
  DisclosureCaret,
  Icon,
  IconButton,
  LinkButton,
  Segmented,
  Spinner,
  StatusDot,
  Switch,
  TextArea,
  TextField,
} from "@/components/controls";
import {
  Callout,
  EmptyState,
  FormRow,
  Group,
  PageHeader,
  Row,
  RowValue,
  SearchField,
  Section,
  StatusTile,
  Tag,
  TagInput,
} from "@/components/layout";
import { ConfirmHost, Dialog, FooterSpacer, Menu, PopupSelect, Sheet, SheetHeader, ToastHost, confirm, toast, useMenu } from "@/components/overlay";
import { Card, Chip, ErrorText, HintText, ListRow, StatusLine } from "@/components/patterns";
import { applyPrefs, useApp } from "./store";
import g from "./Gallery.module.css";

type Theme = "light" | "dark";

const params = new URLSearchParams(location.search);

/** Shows the document in this theme and density, without saving either to the app's preferences. */
function show(appearance: Theme, density: "regular" | "compact") {
  applyPrefs({ ...useApp.getState().prefs, appearance, density });
}

/**
 * Every primitive and pattern in `components/`, in each state, for checking a change to them at a glance
 * (`pnpm dev`, then `/?gallery`). Development builds only (main.tsx).
 */
export function Gallery() {
  const embedded = params.has("embed");
  const [mode, setMode] = useState<Theme | "both">(() => (params.get("theme") as Theme | null) ?? "light");
  const [density, setDensityState] = useState<"regular" | "compact">(() =>
    params.get("density") === "compact" ? "compact" : "regular",
  );
  // Side by side, each frame shows its own theme; the toolbar around them stays light.
  useEffect(() => show(mode === "both" ? "light" : mode, density), [mode, density]);

  if (embedded)
    return (
      <div className={g.embedded}>
        <Specimens />
      </div>
    );
  return (
    <div className={g.page}>
      <div className={g.toolbar}>
        <span className={g.title}>Component gallery</span>
        <Segmented
          value={mode}
          ariaLabel="Theme"
          options={[
            { value: "light", label: "Light", icon: "sun" },
            { value: "dark", label: "Dark", icon: "moon" },
            { value: "both", label: "Side by side", icon: "columns" },
          ]}
          onChange={setMode}
        />
        <Segmented
          value={density}
          ariaLabel="Density"
          options={[
            { value: "regular", label: "Regular" },
            { value: "compact", label: "Compact" },
          ]}
          onChange={setDensityState}
        />
      </div>
      {mode === "both" ? (
        <div className={g.split}>
          {(["light", "dark"] as const).map((theme) => (
            <iframe key={`${theme}-${density}`} title={theme} className={g.frame} src={`?gallery&embed&theme=${theme}&density=${density}`} />
          ))}
        </div>
      ) : (
        <div className={g.scroll}>
          <Specimens />
        </div>
      )}
    </div>
  );
}

function Block({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className={g.block}>
      <h2 className={g.blockTitle}>{title}</h2>
      {children}
    </section>
  );
}

/** One labelled line of specimens. */
function Line({ label, children, column }: { label: string; children: ReactNode; column?: boolean }) {
  return (
    <div className={g.line}>
      <span className={g.label}>{label}</span>
      <div className={column ? g.column : g.items}>{children}</div>
    </div>
  );
}

function Specimens() {
  return (
    <div className={g.specimens}>
      <Buttons />
      <Fields />
      <Choices />
      <Indicators />
      <Layout />
      <Patterns />
      <Overlays />
      <ConfirmHost />
      <ToastHost />
    </div>
  );
}

function Buttons() {
  return (
    <Block title="Buttons">
      {(["md", "sm", "xs"] as const).map((size) => (
        <Line key={size} label={`Button ${size}`}>
          <Button size={size}>Default</Button>
          <Button size={size} variant="primary">
            Primary
          </Button>
          <Button size={size} variant="danger">
            Delete
          </Button>
          <Button size={size} icon="plus">
            With icon
          </Button>
          <Button size={size} trailingIcon="arrow-up-right">
            External
          </Button>
          <Button size={size} disabled>
            Disabled
          </Button>
          <Button size={size} busy>
            Busy
          </Button>
          <Button size={size} variant="primary" icon="arrow-clockwise" busy>
            Busy with icon
          </Button>
        </Line>
      ))}
      <Line label="LinkButton">
        <LinkButton>Accent</LinkButton>
        <LinkButton icon="arrow-square-out">With icon</LinkButton>
        <LinkButton tone="muted">Muted</LinkButton>
        <LinkButton tone="danger">Danger</LinkButton>
        <LinkButton disabled>Disabled</LinkButton>
      </Line>
      <Line label="IconButton">
        <IconButton icon="pencil-simple" label="Edit" />
        <IconButton icon="sidebar" label="Active" active />
        <IconButton icon="trash" label="Disabled" disabled />
      </Line>
    </Block>
  );
}

function Fields() {
  const [text, setText] = useState("deploy@prod-api");
  const [tags, setTags] = useState(["production", "api"]);
  const [search, setSearch] = useState("");
  return (
    <Block title="Fields">
      <Line label="TextField">
        <TextField placeholder="Placeholder" />
        <TextField value={text} onChange={(e) => setText(e.target.value)} />
        <TextField mono defaultValue="127.0.0.1:5432" />
        <TextField large defaultValue="Large" />
        <TextField invalid defaultValue="Invalid" />
        <TextField secret defaultValue="hunter2" />
        <TextField leading={<Icon name="globe" />} trailing="ms" defaultValue="250" />
        <TextField disabled defaultValue="Disabled" />
      </Line>
      <Line label="SearchField">
        <SearchField value={search} onChange={setSearch} placeholder="Search hosts" shortcut="Ctrl K" />
      </Line>
      <Line label="TextArea" column>
        <TextArea defaultValue={"Multi-line text\nwraps and resizes"} />
      </Line>
      <Line label="TagInput" column>
        <TagInput value={tags} onChange={setTags} placeholder="Add a tag" />
      </Line>
      <Line label="Field messages" column>
        <HintText>Up to 64 characters. Letters, digits and dashes.</HintText>
        <ErrorText>The two passphrases don't match.</ErrorText>
      </Line>
    </Block>
  );
}

function Choices() {
  const [seg, setSeg] = useState("list");
  const [on, setOn] = useState(true);
  const [checked, setChecked] = useState(true);
  const [pick, setPick] = useState("ed25519");
  return (
    <Block title="Choices">
      <Line label="Segmented">
        <Segmented
          value={seg}
          onChange={setSeg}
          options={[
            { value: "list", label: "List", icon: "list" },
            { value: "cards", label: "Cards", icon: "squares-four" },
            { value: "off", label: "Unavailable", disabled: true },
          ]}
        />
      </Line>
      <Line label="Switch">
        <Switch checked={on} onChange={setOn} label="On" />
        <Switch checked={!on} onChange={(v) => setOn(!v)} label="Off" />
        <Switch checked disabled onChange={() => {}} label="Disabled on" />
        <Switch checked={false} disabled onChange={() => {}} label="Disabled off" />
      </Line>
      <Line label="Checkbox">
        <Checkbox checked={checked} onChange={setChecked}>
          Checked
        </Checkbox>
        <Checkbox checked={!checked} onChange={(v) => setChecked(!v)}>
          Unchecked
        </Checkbox>
      </Line>
      <Line label="PopupSelect">
        <PopupSelect
          value={pick}
          onChange={setPick}
          icon="key"
          options={[
            { value: "ed25519", label: "deploy-ed25519", hint: "ED25519" },
            { value: "rsa", label: "bastion-rsa", hint: "RSA 4096" },
          ]}
        />
        <PopupSelect value="a" onChange={() => {}} disabled options={[{ value: "a", label: "Disabled" }]} />
      </Line>
      <Line label="DisclosureCaret">
        <DisclosureCaret open={false} />
        <DisclosureCaret open />
      </Line>
    </Block>
  );
}

function Indicators() {
  return (
    <Block title="Indicators">
      <Line label="StatusDot">
        <StatusDot color="var(--green)" />
        <StatusDot color="var(--orange)" />
        <StatusDot color="var(--red)" />
        <StatusDot color="var(--fg3)" />
      </Line>
      <Line label="Spinner">
        <Spinner />
        <Spinner size={11} />
      </Line>
      <Line label="Badge · Tag">
        <Badge>ED25519</Badge>
        <Tag name="production" />
        <Tag name="staging" />
      </Line>
      <Line label="StatusTile">
        <StatusTile icon="cloud-check" color="var(--green)" />
        <StatusTile icon="warning" color="var(--orange)" />
        <StatusTile icon="x-circle" color="var(--red)" />
      </Line>
      <Line label="Chip">
        <Chip>Queued</Chip>
        <Chip tone="success" icon="check">
          Done
        </Chip>
        <Chip tone="error">Exit 5</Chip>
        <Chip tone="warning">Needs approval</Chip>
        <Chip tone="accent" icon="lightning">
          Auto
        </Chip>
        <Chip busy>Running</Chip>
        <Chip mono>Ctrl+C</Chip>
      </Line>
      <Line label="StatusLine" column>
        <StatusLine tone="success">localhost:15432</StatusLine>
        <StatusLine tone="progress">Starting…</StatusLine>
        <StatusLine tone="warning">Host key changed</StatusLine>
        <StatusLine tone="error">
          channel open failed: administratively prohibited: open failed (the server does not allow forwarding to this address)
        </StatusLine>
        <StatusLine>Not started</StatusLine>
      </Line>
    </Block>
  );
}

function Layout() {
  return (
    <Block title="Layout">
      <Line label="PageHeader" column>
        <div className={g.framed}>
          <PageHeader title="All Hosts" count="11 hosts">
            <Button>Import</Button>
            <Button variant="primary" icon="plus">
              New Host
            </Button>
          </PageHeader>
        </div>
      </Line>
      <Line label="Section · Group" column>
        <Section title="Connection">
          <Group>
            <Row label="Address">
              <RowValue mono>43.206.118.27</RowValue>
            </Row>
            <Row label="Keep alive">
              <Switch checked onChange={() => {}} label="Keep alive" />
            </Row>
          </Group>
        </Section>
      </Line>
      <Line label="FormRow" column>
        <Group>
          <FormRow label="Name">
            <TextField defaultValue="prod-api-tokyo" />
          </FormRow>
          <FormRow label="Port" error="Enter a port from 1 to 65535.">
            <TextField invalid defaultValue="70000" />
          </FormRow>
        </Group>
      </Line>
      <Line label="Callout" column>
        <Callout icon="info" title="Synced with Cloudflare D1">
          Changes on this device reach your other devices within a minute.
        </Callout>
        <Callout icon="warning" iconColor="var(--orange)">
          The server's host key changed since the last connection.
        </Callout>
      </Line>
      <Line label="Card" column>
        <div className={g.cards}>
          <Card>
            <strong>Card</strong>
            <HintText>A raised panel that stands on its own.</HintText>
          </Card>
          <Card interactive>
            <strong>Interactive card</strong>
            <HintText>Lightens under the pointer.</HintText>
          </Card>
        </div>
      </Line>
      <Line label="EmptyState" column>
        <div className={g.framed}>
          <EmptyState
            icon="arrows-left-right"
            title="No port forwards"
            body="Forward a port to reach a service on the server."
            actions={<Button variant="primary">Add Forward…</Button>}
          />
        </div>
      </Line>
    </Block>
  );
}

function Patterns() {
  const [selected, setSelected] = useState(1);
  const actions = (
    <>
      <Button size="xs">Stop</Button>
      <IconButton icon="pencil-simple" label="Edit" />
    </>
  );
  return (
    <Block title="List rows">
      <Line label="ListRow" column>
        <Group>
          <ListRow status={<StatusLine tone="success">localhost:15432</StatusLine>} trailing={actions}>
            <span className={g.mono}>127.0.0.1:15432 → 127.0.0.1:5432</span>
          </ListRow>
          <ListRow status={<StatusLine tone="progress">Starting…</StatusLine>} trailing={actions}>
            <span className={g.mono}>127.0.0.1:3000 → grafana.internal:3000</span>
          </ListRow>
          <ListRow status={<StatusLine tone="error">Address already in use (os error 10048)</StatusLine>} trailing={actions}>
            <span className={g.mono}>127.0.0.1:1080 SOCKS proxy</span>
          </ListRow>
        </Group>
      </Line>
      <Line label="Interactive · selected" column>
        <Group>
          {["prod-api-tokyo", "db-osaka-01", "staging-web-02"].map((name, i) => (
            <ListRow
              key={name}
              interactive
              selected={i === selected}
              leading={<Icon name="hard-drives" size={16} />}
              onClick={() => setSelected(i)}
            >
              {name}
            </ListRow>
          ))}
        </Group>
      </Line>
      <Line label="ListRow sm" column>
        <div className={g.popover}>
          <ListRow
            size="sm"
            leading={<Icon name="check-circle" fill color="var(--green)" />}
            trailing={<LinkButton>Stop</LinkButton>}
          >
            <span className={g.mono}>localhost:15432</span>
          </ListRow>
          <ListRow size="sm" leading={<Spinner size={13} />} trailing={<LinkButton disabled>Start</LinkButton>}>
            <span className={g.mono}>localhost:3000</span>
          </ListRow>
        </div>
      </Line>
    </Block>
  );
}

function Overlays() {
  const menu = useMenu();
  const [open, setOpen] = useState<"sheet" | "dialog" | null>(null);
  return (
    <Block title="Overlays">
      <Line label="Menu">
        <Button onClick={(e) => menu.openBelow(e.currentTarget)} aria-expanded={!!menu.anchor}>
          Open menu
        </Button>
        {menu.anchor && (
          <Menu
            anchor={menu.anchor}
            onClose={menu.close}
            entries={[
              { kind: "header", label: "Host" },
              { label: "Connect", icon: "terminal-window", hint: "Enter", onSelect: () => {} },
              { label: "Favorite", icon: "star", checked: true, onSelect: () => {} },
              { label: "Unavailable", icon: "prohibit", disabled: true, onSelect: () => {} },
              { kind: "separator" },
              { label: "Delete…", icon: "trash", danger: true, onSelect: () => {} },
            ]}
          />
        )}
      </Line>
      <Line label="Sheet · Dialog · confirm">
        <Button onClick={() => setOpen("sheet")}>Open sheet</Button>
        <Button onClick={() => setOpen("dialog")}>Open dialog</Button>
        <Button
          variant="danger"
          onClick={() => void confirm({ title: "Delete prod-api-tokyo?", body: "This can't be undone.", confirmLabel: "Delete", danger: true })}
        >
          confirm()
        </Button>
      </Line>
      <Line label="toast">
        <Button onClick={() => toast("Copied localhost:15432")}>Info</Button>
        <Button onClick={() => toast("Forward started", "success")}>Success</Button>
        <Button onClick={() => toast("Couldn't reach the server", "error")}>Error</Button>
      </Line>
      {open === "sheet" && (
        <Sheet
          onClose={() => setOpen(null)}
          footer={
            <>
              <Button>Back</Button>
              <FooterSpacer />
              <Button onClick={() => setOpen(null)}>Cancel</Button>
              <Button variant="primary" onClick={() => setOpen(null)}>
                Continue
              </Button>
            </>
          }
        >
          <SheetHeader title="Set up sync" subtitle="Keep your vault on your own Cloudflare account." />
        </Sheet>
      )}
      {open === "dialog" && (
        <Dialog
          title="Verify your password"
          body="Enter the vault password to show the recovery code."
          icon="lock"
          onClose={() => setOpen(null)}
          actions={
            <>
              <Button onClick={() => setOpen(null)}>Cancel</Button>
              <Button variant="primary" onClick={() => setOpen(null)}>
                Verify
              </Button>
            </>
          }
        />
      )}
    </Block>
  );
}
