import { version as APP_VERSION } from "../../package.json";

export function GuideEn() {
  return (
    <div className="page guide">
      <h2>Guide — How does it work?</h2>
      <p className="page__subtitle">PartaGPU v{APP_VERSION}</p>

      <section className="guide__section">
        <h3>1. Create or join a room</h3>
        <p>
          A <strong>room</strong> is a shared cryptographic enclosure: every
          machine that belongs to it can dispatch compute between members,
          and machines outside the room are automatically refused.
        </p>
        <ul>
          <li>
            <strong>Create a room</strong> (one student does it): click{" "}
            <em>"Create a room"</em>, choose a name (e.g.{" "}
            <em>"Room B204"</em>). The app generates a 4-word access code
            (<code>apple-tiger-blue-ocean</code>). The code is shown masked
            (<code>*****-*****-****-*****</code>); hold the eye icon to
            reveal it just long enough to read it aloud.
          </li>
          <li>
            <strong>Join a room</strong> (everyone else): click{" "}
            <em>"Join a room"</em>, enter the same name and the dictated
            4-word code.
          </li>
        </ul>
        <p>
          Verified peers display a green <strong>OK</strong> badge.
          Peers from another room (or no room) display a red{" "}
          <strong>?</strong> — you cannot dispatch tasks to them.
        </p>
      </section>

      <section className="guide__section">
        <h3>2. Enable sharing on this machine</h3>
        <p>
          <strong>"My sharing"</strong> tab → <em>"Enable"</em> button. A
          PolicyKit window asks for the admin password. The app creates the{" "}
          <code>partagpu</code> system account, configures its cgroup v2,
          and opens the firewall on the required ports.
        </p>
        <p>
          Then set the <strong>partagpu account password</strong> in the
          form that appears. This password lets people log into the machine
          from the system login screen (useful for absent classmates'
          computers — see below).
        </p>
      </section>

      <section className="guide__section">
        <h3>3. Adjust sharing limits</h3>
        <p>
          Each resource gauge (CPU, RAM, GPU) has a{" "}
          <strong>draggable red cursor</strong> showing the limit you share.
          Drag it with the mouse to adjust — the change is applied
          instantly via the kernel's cgroup v2, no password required.
        </p>
        <p>
          The <strong>"Max concurrent tasks"</strong> field (default 4)
          caps how many tasks can run at the same time. Beyond it, new
          tasks queue up — protection against a peer trying to flood your
          machine.
        </p>
      </section>

      <section className="guide__section">
        <h3>4. Submit tasks</h3>
        <p>
          <strong>"My usage"</strong> tab. Three ways to dispatch compute:
        </p>
        <ul>
          <li>
            <strong>A command on a peer</strong>: a form that takes a shell
            command or an uploaded Python script. Ideal for a one-off test
            or a non-distributed job.
          </li>
          <li>
            <strong>Multi-machine DDP training</strong>: a dedicated panel
            where you tick several peers and their GPUs, upload a PyTorch
            script; the app automatically computes <code>MASTER_ADDR</code>,{" "}
            <code>RANK</code>, <code>WORLD_SIZE</code>,{" "}
            <code>CUDA_VISIBLE_DEVICES</code>, etc. Live per-rank progress
            table; <em>"Cancel all"</em> button propagates the abort to
            every rank if one crashes.
          </li>
          <li>
            <strong>From Python</strong>:{" "}
            <code>pip install partagpu</code>, then{" "}
            <code>partagpu.run_remote(...)</code> or{" "}
            <code>partagpu.distribute("train.py")</code> in a notebook.
            See the Python package documentation.
          </li>
        </ul>
      </section>

      <section className="guide__section">
        <h3>5. Real-time monitoring</h3>
        <ul>
          <li>
            <strong>Task table</strong>: status, progress, CPU/RAM/GPU per
            task, updated every second via Tauri events (no visible
            polling).
          </li>
          <li>
            <strong>Stop button</strong> on each running task: clean
            SIGTERM on the peer side, SIGKILL after 2 s. For DDP jobs,
            cancelling one rank propagates to all the others to avoid an
            NCCL hang.
          </li>
          <li>
            <strong>Desktop notifications</strong>: when a remote dispatch
            ends (Completed / Failed / Cancelled), a native system toast
            shows up — even if the app is unfocused. Convenient for
            stepping away during a long training run.
          </li>
          <li>
            <strong>5-minute history</strong>: under the gauges on{" "}
            <em>"My sharing"</em>, sparklines show CPU / RAM / GPU
            evolution over the last 5 minutes.
          </li>
        </ul>
      </section>

      <section className="guide__section">
        <h3>6. Fleet view</h3>
        <p>
          <strong>"Fleet view"</strong> tab: an aggregate dashboard of every
          machine in the room. Global stats up top (visible peers,
          available GPUs, my total usage) and one card per peer with its
          offered capacity and the tasks you are running on it. Useful for
          a teacher supervising the room.
        </p>
      </section>

      <section className="guide__section">
        <h3>7. Pause vs Disable</h3>
        <p>Two stop levels with different semantics:</p>
        <div className="guide__comparison">
          <div className="guide__card">
            <h4>Pause (temporary)</h4>
            <ul>
              <li>Firewall closed, no more incoming tasks</li>
              <li><code>partagpu</code> account kept</li>
              <li>Cgroup kept</li>
              <li>Managed venv kept</li>
              <li>
                <em>Resume</em> = instant, no password requested
              </li>
            </ul>
          </div>
          <div className="guide__card">
            <h4>Disable (full cleanup)</h4>
            <ul>
              <li>Running tasks <strong>killed</strong> (pkill -u partagpu)</li>
              <li>
                <code>partagpu</code> account <strong>deleted</strong>{" "}
                (userdel --remove)
              </li>
              <li>
                Managed venv <strong>removed</strong> (~3 GB freed)
              </li>
              <li>Cgroup and SSH/sudo deny rules removed</li>
              <li>
                <em>Enable</em> asks for pkexec and re-downloads the venv
                if wanted (~5 min)
              </li>
            </ul>
          </div>
        </div>
        <p>
          Pick <em>Pause</em> to end the day's session. Pick{" "}
          <em>Disable</em> to fully release the machine of any PartaGPU
          residue.
        </p>
      </section>

      <section className="guide__section">
        <h3>8. Enable sharing on an absent classmate's computer</h3>
        <ol>
          <li>Turn on the absent classmate's computer.</li>
          <li>
            On the login screen (GDM, LightDM…), pick the{" "}
            <code>partagpu</code> user.
          </li>
          <li>
            Enter the <strong>shared password</strong> set during initial
            setup.
          </li>
          <li>PartaGPU starts automatically.</li>
          <li>
            Join the room (a classmate dictates the access code — or you
            can read it on your own machine by holding the eye icon).
          </li>
          <li>
            Click <em>"Enable sharing"</em> — no pkexec prompt, the
            account is already in place.
          </li>
        </ol>
      </section>

      <section className="guide__section">
        <h3>9. Security</h3>
        <ul>
          <li>
            <strong>HMAC authentication</strong>: every peer request carries
            a <code>X-PartaGPU-AUTH: &lt;ts&gt;:&lt;HMAC&gt;</code> header
            that binds auth to the body + a timestamp inside a 30 s window
            (anti-replay). A different room = different HMAC = request
            rejected before even attempting decryption.
          </li>
          <li>
            <strong>AES-256-GCM encryption</strong> with{" "}
            <strong>forward secrecy</strong>: session key derived from an
            ephemeral X25519 Diffie-Hellman exchange per request. The
            server key lives in RAM only and rotates every 10 minutes.
          </li>
          <li>
            <strong>Bubblewrap sandbox</strong> per task: read-only file
            system, isolated PID namespace, network unshared by default,
            command allowlist, dedicated sub-cgroup so one task cannot
            OOM the others.
          </li>
          <li>
            <strong>Hardened partagpu account</strong>: SSH blocked, sudo
            blocked, restricted shell that only launches PartaGPU.
          </li>
          <li>
            <strong>Masked access code</strong>: the passphrase is never
            displayed in clear by default — you must hold the eye icon to
            reveal it.
          </li>
        </ul>
      </section>

      <section className="guide__section guide__section--tip">
        <h3>Good to know</h3>
        <ul>
          <li>
            <strong>No NVIDIA GPU</strong>? No problem, CPU + RAM sharing
            works regardless. The GPU gauge stays hidden.
          </li>
          <li>
            <strong>No visible machine</strong>? Check that you're on the
            same subnet and that multicast is allowed (UDP 5353 for mDNS).
            See <code>docs/TROUBLESHOOTING.en.md</code> for the rest.
          </li>
          <li>
            <strong>ML toolkit</strong> (torch, torchvision, scipy, pandas,
            etc.) installable in one click from <em>"My sharing"</em> →{" "}
            <em>Python environment</em>. ~3 GB, 5–10 min download. Avoids
            having to touch the system Python.
          </li>
          <li>
            <strong>Detailed documentation</strong> in the repo:{" "}
            <code>README.en.md</code>, <code>docs/ARCHITECTURE.en.md</code>,{" "}
            <code>SECURITY.en.md</code>,{" "}
            <code>docs/TROUBLESHOOTING.en.md</code>.
          </li>
        </ul>
      </section>
    </div>
  );
}
