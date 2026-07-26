import "./styles.css";

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { confirm, open, save } from "@tauri-apps/plugin-dialog";

type RenderingIntent =
  | "perceptual"
  | "relative_colorimetric"
  | "saturation"
  | "absolute_colorimetric";

interface EngineInfo {
  name: string;
  version: string;
  executable: string;
}

interface EngineAvailability {
  available: boolean;
  info: EngineInfo | null;
  error: string | null;
}

interface FontInfo {
  page_number: number;
  resource_name: string;
  base_name: string | null;
  subtype: string | null;
  embedded: boolean;
  subset: boolean;
  has_to_unicode: boolean;
}

interface ImageInfo {
  page_number: number;
  resource_name: string;
  color_space: string;
}

interface TransparencyFinding {
  page_number: number;
  kind: string;
  detail: string;
}

interface OutputIntentInfo {
  output_condition_identifier: string | null;
  info: string | null;
  profile: { sha256: string; color_space: string | null } | null;
}

interface PdfInspectionResult {
  path: string;
  pdf_version: string;
  pdfx_claim: string | null;
  page_count: number;
  fonts: FontInfo[];
  images: ImageInfo[];
  transparency: TransparencyFinding[];
  output_intent: OutputIntentInfo | null;
}

interface PreflightFinding {
  rule: string;
  severity: "info" | "warning" | "error";
  page_number: number | null;
  message: string;
}

interface InspectionReport {
  document: PdfInspectionResult;
  findings: PreflightFinding[];
  engine: EngineAvailability;
}

interface ValidationResult {
  passed: boolean;
  page_count_matches: boolean;
  page_boxes_match: boolean;
  fonts_remaining: number;
  type3_fonts_remaining: number;
  rgb_images_remaining: number;
  cmyk_images: number;
  image_content_retained: boolean;
  transparency_remaining: number;
  output_intent_matches: boolean;
  errors: string[];
}

interface FullConversionReport {
  input: PdfInspectionResult;
  conversion: {
    output_path: string;
    duration_ms: number;
    flattened_pages: number[];
    engine_warnings: string[];
  };
  output: PdfInspectionResult;
  validation: ValidationResult;
}

interface ConversionOptions {
  input_path: string;
  output_path: string;
  kind: "outline" | "pdf_x1a";
  destination_icc: string | null;
  rendering_intent: RenderingIntent;
  black_point_compensation: boolean;
  outline_fonts: boolean;
  transparency_resolution_dpi: number;
  allow_transparency: boolean;
}

const app = document.querySelector<HTMLDivElement>("#app");
if (!app) throw new Error("#app was not found");

app.innerHTML = `
  <header class="app-header">
    <div class="brand-mark" aria-hidden="true"><span>O</span><span>P</span></div>
    <div>
      <p class="eyebrow">PRINT PRODUCTION UTILITY</p>
      <h1>Open Print PDF</h1>
    </div>
    <span id="app-version" class="app-version">versionを確認中</span>
    <div id="engine-status" class="engine-pill pending">変換エンジンを確認中</div>
  </header>

  <main>
    <section id="drop-zone" class="drop-zone" tabindex="0" role="button">
      <div class="drop-icon">PDF</div>
      <div>
        <h2>PDFをここへドロップ</h2>
        <p>PDF/X情報、フォント、画像色空間、透過をGhostscriptなしで検査します。</p>
      </div>
      <button id="choose-pdf" class="button secondary" type="button">PDFを選択</button>
    </section>

    <section id="empty-state" class="empty-state">
      <p>検査するPDFを選ぶと、ここにプリフライト結果が表示されます。</p>
    </section>

    <div id="workspace" class="workspace hidden">
      <div class="main-column">
        <section class="panel document-panel">
          <div class="panel-heading">
            <div>
              <p class="eyebrow">INPUT DOCUMENT</p>
              <h2 id="file-name"></h2>
              <p id="file-path" class="path-text"></p>
            </div>
            <button id="open-input" class="text-button" type="button">PDFを開く</button>
          </div>
          <div id="summary-cards" class="summary-grid"></div>
        </section>

        <section class="panel">
          <div class="panel-heading compact">
            <div>
              <p class="eyebrow">PREFLIGHT</p>
              <h2>検査結果</h2>
            </div>
            <span id="finding-count" class="count-badge"></span>
          </div>
          <div id="findings" class="findings"></div>
        </section>

        <section class="panel">
          <div class="panel-heading compact">
            <div>
              <p class="eyebrow">FONT REPORT</p>
              <h2>フォント</h2>
            </div>
            <span id="type3-count" class="count-badge"></span>
          </div>
          <div class="table-scroll">
            <table>
              <thead><tr><th>フォント名</th><th>種類</th><th>ページ</th><th>埋め込み</th></tr></thead>
              <tbody id="font-table"></tbody>
            </table>
          </div>
        </section>

        <section id="result-panel" class="panel result-panel hidden">
          <div class="result-heading">
            <div class="result-check">✓</div>
            <div><p class="eyebrow">VALIDATION PASSED</p><h2>変換が完了しました</h2></div>
            <div class="result-actions">
              <button id="open-output" class="button primary" type="button">変換後PDFを開く</button>
              <button id="open-folder" class="button secondary" type="button">エクスプローラーで表示</button>
            </div>
          </div>
          <div id="comparison" class="comparison"></div>
        </section>
      </div>

      <aside class="side-column">
        <section class="panel settings-panel">
          <p class="eyebrow">CONVERSION</p>
          <h2>出力設定</h2>

          <div class="conversion-actions">
            <button id="convert-pdfx" class="button primary wide" type="button">PDF/X-1aへ変換</button>
            <p class="fine-print">変換後はフォント、RGB、透過、ページボックス、OutputIntentを自動検証します。</p>
          </div>

          <label class="field"><span>レンダリングインテント</span>
            <select id="intent">
              <option value="relative_colorimetric">相対的な色域を維持</option>
              <option value="perceptual">知覚的</option>
              <option value="saturation">彩度</option>
              <option value="absolute_colorimetric">絶対的な色域を維持</option>
            </select>
          </label>

          <label class="check-field"><input id="bpc" type="checkbox" checked /><span>黒点補正を使用</span></label>
          <label class="check-field"><input id="outline-fonts" type="checkbox" checked /><span>フォントをアウトライン化</span></label>

          <label class="field"><span>透過フラット化解像度</span>
            <div class="input-suffix"><input id="dpi" type="number" min="72" max="2400" value="720" /><span>dpi</span></div>
          </label>

          <div class="field">
            <span>出力ICCプロファイル</span>
            <button id="choose-icc" class="file-choice" type="button"><strong>入力PDFのOutputIntent</strong><small id="icc-name">埋め込みプロファイルを再利用</small></button>
            <button id="clear-icc" class="text-button hidden" type="button">入力プロファイルへ戻す</button>
          </div>

          <div id="transparency-note" class="notice warning hidden"></div>
          <div id="engine-note" class="notice hidden"></div>
        </section>
      </aside>
    </div>
  </main>

  <div id="busy" class="busy hidden" role="status" aria-live="polite">
    <div class="spinner"></div><strong id="busy-title">処理中</strong><span id="busy-detail"></span>
  </div>
  <div id="toast" class="toast hidden" role="alert" aria-live="assertive">
    <div class="toast-content">
      <strong id="toast-title"></strong>
      <pre id="toast-message"></pre>
    </div>
    <button id="toast-close" class="toast-close hidden" type="button" aria-label="メッセージを閉じる">閉じる</button>
  </div>
`;

const element = <T extends HTMLElement>(selector: string): T => {
  const value = document.querySelector<T>(selector);
  if (!value) throw new Error(`${selector} was not found`);
  return value;
};

let inspection: InspectionReport | null = null;
let outputPath: string | null = null;
let selectedIcc: string | null = null;
let busy = false;
let toastTimer: number | null = null;

const escapeHtml = (value: string): string =>
  value.replace(/[&<>'"]/g, (character) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;",
  })[character] ?? character);

const fileName = (path: string): string => path.split(/[\\/]/).pop() || path;
const parentDirectory = (path: string): string => path.replace(/[\\/][^\\/]+$/, "") || path;
const rgbCount = (document: PdfInspectionResult): number =>
  document.images.filter((image) => image.color_space.includes("rgb")).length;
const cmykCount = (document: PdfInspectionResult): number =>
  document.images.filter((image) => image.color_space.includes("cmyk")).length;

function setBusy(active: boolean, title = "処理中", detail = ""): void {
  busy = active;
  element<HTMLDivElement>("#busy").classList.toggle("hidden", !active);
  element<HTMLElement>("#busy-title").textContent = title;
  element<HTMLElement>("#busy-detail").textContent = detail;
  document.querySelectorAll<HTMLButtonElement>("button").forEach((button) => {
    if (button.id !== "open-input") button.disabled = active;
  });
  updateEngineControls();
}

function showToast(message: string, isError = false): void {
  const toast = element<HTMLDivElement>("#toast");
  if (toastTimer !== null) {
    window.clearTimeout(toastTimer);
    toastTimer = null;
  }
  const displayedVersion = element<HTMLElement>("#app-version").textContent;
  element<HTMLElement>("#toast-title").textContent = isError
    ? `処理に失敗しました（${displayedVersion}）`
    : "完了";
  element<HTMLElement>("#toast-message").textContent = message;
  element<HTMLButtonElement>("#toast-close").classList.toggle("hidden", !isError);
  toast.classList.toggle("error", isError);
  toast.classList.remove("hidden");
  if (!isError) {
    toastTimer = window.setTimeout(() => {
      toast.classList.add("hidden");
      toastTimer = null;
    }, 5500);
  }
}

function updateEngineControls(): void {
  const available = inspection?.engine.available ?? false;
  element<HTMLButtonElement>("#convert-pdfx").disabled = busy || !available;
}

function renderEngine(engine: EngineAvailability): void {
  const pill = element<HTMLDivElement>("#engine-status");
  pill.className = `engine-pill ${engine.available ? "ready" : "unavailable"}`;
  pill.textContent = engine.available
    ? `Ghostscript ${engine.info?.version ?? "検出済み"}`
    : "変換エンジンの修復が必要";
  const note = element<HTMLDivElement>("#engine-note");
  note.classList.toggle("hidden", engine.available);
  note.className = `notice error ${engine.available ? "hidden" : ""}`;
  note.textContent = engine.error
    ? `同梱Ghostscriptを起動できません。アプリを再インストールしてください。詳細: ${engine.error}`
    : "同梱Ghostscriptが見つかりません。アプリを再インストールしてください。";
  updateEngineControls();
}

function renderInspection(report: InspectionReport): void {
  inspection = report;
  outputPath = null;
  element("#empty-state").classList.add("hidden");
  element("#workspace").classList.remove("hidden");
  element("#result-panel").classList.add("hidden");
  const document = report.document;
  element("#file-name").textContent = fileName(document.path);
  element("#file-path").textContent = document.path;
  const outputIntent = document.output_intent?.output_condition_identifier
    ?? document.output_intent?.info ?? "なし";
  element("#summary-cards").innerHTML = [
    ["PDF", document.pdf_version],
    ["規格", document.pdfx_claim ?? "未指定"],
    ["ページ", String(document.page_count)],
    ["フォント", String(document.fonts.length)],
    ["RGB画像", String(rgbCount(document))],
    ["OutputIntent", outputIntent],
  ].map(([label, value]) => `<div class="summary-card"><span>${escapeHtml(label)}</span><strong>${escapeHtml(value)}</strong></div>`).join("");

  const type3 = document.fonts.filter((font) => font.subtype === "Type3").length;
  element("#type3-count").textContent = `Type 3: ${type3}`;
  element("#font-table").innerHTML = document.fonts.length
    ? document.fonts.map((font) => `<tr>
        <td><strong>${escapeHtml(font.base_name ?? font.resource_name)}</strong><small>${font.subset ? "サブセット" : "フルフォント"}</small></td>
        <td>${escapeHtml(font.subtype ?? "不明")}</td><td>${font.page_number}</td>
        <td><span class="status-dot ${font.embedded ? "ok" : "bad"}"></span>${font.embedded ? "埋め込み" : "未埋め込み"}</td>
      </tr>`).join("")
    : `<tr><td colspan="4" class="muted-cell">フォントリソースはありません</td></tr>`;

  const errors = report.findings.filter((finding) => finding.severity === "error").length;
  const warnings = report.findings.filter((finding) => finding.severity === "warning").length;
  element("#finding-count").textContent = `${errors} エラー / ${warnings} 警告`;
  element("#findings").innerHTML = report.findings.length
    ? report.findings.map((finding) => `<div class="finding ${finding.severity}">
        <span class="finding-symbol">${finding.severity === "error" ? "!" : finding.severity === "warning" ? "△" : "i"}</span>
        <div><strong>${escapeHtml(finding.message)}</strong><small>${escapeHtml(finding.rule)}${finding.page_number ? ` · ${finding.page_number}ページ` : ""}</small></div>
      </div>`).join("")
    : `<div class="finding success"><span class="finding-symbol">✓</span><div><strong>問題は見つかりませんでした</strong></div></div>`;

  const transparencyPages = [...new Set(document.transparency.map((item) => item.page_number))];
  const transparencyNote = element<HTMLDivElement>("#transparency-note");
  transparencyNote.classList.toggle("hidden", transparencyPages.length === 0);
  transparencyNote.innerHTML = transparencyPages.length
    ? `<strong>透過を検出: ${transparencyPages.join(", ")}ページ</strong><span>PDF/X-1a変換時、このページは720dpiのDeviceCMYK画像になります。</span>`
    : "";
  renderEngine(report.engine);
}

async function inspectPath(path: string): Promise<void> {
  if (!path.toLowerCase().endsWith(".pdf")) {
    showToast("PDFファイルを選択してください。", true);
    return;
  }
  setBusy(true, "PDFを検査しています", fileName(path));
  try {
    const report = await invoke<InspectionReport>("inspect_pdf", { path });
    renderInspection(report);
  } catch (error) {
    showToast(String(error), true);
  } finally {
    setBusy(false);
  }
}

async function choosePdf(): Promise<void> {
  const path = await open({ multiple: false, filters: [{ name: "PDF", extensions: ["pdf"] }] });
  if (typeof path === "string") await inspectPath(path);
}

async function chooseIcc(): Promise<void> {
  const path = await open({ multiple: false, filters: [{ name: "ICC profile", extensions: ["icc", "icm"] }] });
  if (typeof path !== "string") return;
  selectedIcc = path;
  element("#icc-name").textContent = fileName(path);
  element("#clear-icc").classList.remove("hidden");
}

async function runConversion(): Promise<void> {
  if (!inspection || !inspection.engine.available) return;
  const transparencyPages = [...new Set(inspection.document.transparency.map((item) => item.page_number))];
  let allowTransparency = true;
  if (transparencyPages.length) {
    allowTransparency = await confirm(
      `${transparencyPages.join(", ")}ページに透過があります。該当ページは指定解像度の8bit DeviceCMYK画像になり、文字と線画も画像化されます。変換を続けますか？`,
      { title: "透過ページのフラット化", kind: "warning" },
    );
    if (!allowTransparency) return;
  }
  const suggested = inspection.document.path.replace(/\.pdf$/i, "-pdfx1a.pdf");
  const destination = await save({ defaultPath: suggested, filters: [{ name: "PDF", extensions: ["pdf"] }] });
  if (!destination) return;
  const options: ConversionOptions = {
    input_path: inspection.document.path,
    output_path: destination,
    kind: "pdf_x1a",
    destination_icc: selectedIcc,
    rendering_intent: element<HTMLSelectElement>("#intent").value as RenderingIntent,
    black_point_compensation: element<HTMLInputElement>("#bpc").checked,
    outline_fonts: element<HTMLInputElement>("#outline-fonts").checked,
    transparency_resolution_dpi: Number(element<HTMLInputElement>("#dpi").value),
    allow_transparency: allowTransparency,
  };
  setBusy(true, "PDF/X-1aへ変換しています", "変換後に自動検証します");
  try {
    const report = await invoke<FullConversionReport>("convert_pdf", { options });
    outputPath = report.conversion.output_path;
    renderConversion(report, options.outline_fonts);
    showToast("変換と検証が完了しました。");
  } catch (error) {
    showToast(String(error), true);
  } finally {
    setBusy(false);
  }
}

function renderConversion(
  report: FullConversionReport,
  outlineFonts: boolean,
): void {
  const input = report.input;
  const output = report.output;
  const rows: Array<[string, string, string, boolean]> = [
    ["ページ", String(input.page_count), String(output.page_count), report.validation.page_count_matches],
    ["ページボックス", "入力値", "変更なし", report.validation.page_boxes_match],
    ["フォント", String(input.fonts.length), String(output.fonts.length), !outlineFonts || report.validation.fonts_remaining === 0],
    ["Type 3", String(input.fonts.filter((font) => font.subtype === "Type3").length), String(report.validation.type3_fonts_remaining), !outlineFonts || report.validation.type3_fonts_remaining === 0],
    ["RGB画像", String(rgbCount(input)), String(report.validation.rgb_images_remaining), report.validation.rgb_images_remaining === 0],
    ["DeviceCMYK画像", String(cmykCount(input)), String(report.validation.cmyk_images), true],
    ["画像内容", "入力画像", report.validation.image_content_retained ? "保持" : "大幅に減少", report.validation.image_content_retained],
    ["PDF/X", input.pdfx_claim ?? "なし", output.pdfx_claim ?? "なし", output.pdfx_claim?.includes("PDF/X-1") ?? false],
    ["OutputIntent", input.output_intent?.output_condition_identifier ?? "なし", report.validation.output_intent_matches ? "一致" : "不一致", report.validation.output_intent_matches],
    ["フラット化ページ", "—", report.conversion.flattened_pages.length ? report.conversion.flattened_pages.join(", ") : "なし", true],
  ];
  element("#comparison").innerHTML = rows.map(([label, before, after, passed]) => `<div class="comparison-row">
    <strong>${escapeHtml(label)}</strong><span>${escapeHtml(before)}</span><b>→</b><span>${escapeHtml(after)}</span><i class="${passed ? "pass" : "fail"}">${passed ? "✓" : "!"}</i>
  </div>`).join("");
  element("#result-panel").classList.remove("hidden");
  element("#result-panel").scrollIntoView({ behavior: "smooth", block: "start" });
}

async function openExistingPath(path: string | null): Promise<void> {
  if (!path) return;
  try {
    await invoke("open_path", { path });
  } catch (error) {
    showToast(String(error), true);
  }
}

element("#choose-pdf").addEventListener("click", () => void choosePdf());
element("#drop-zone").addEventListener("dblclick", () => void choosePdf());
element("#drop-zone").addEventListener("keydown", (event) => {
  if ((event as KeyboardEvent).key === "Enter") void choosePdf();
});
element("#choose-icc").addEventListener("click", () => void chooseIcc());
element("#clear-icc").addEventListener("click", () => {
  selectedIcc = null;
  element("#icc-name").textContent = "埋め込みプロファイルを再利用";
  element("#clear-icc").classList.add("hidden");
});
element("#convert-pdfx").addEventListener("click", () => void runConversion());
element("#open-input").addEventListener("click", () => void openExistingPath(inspection?.document.path ?? null));
element("#open-output").addEventListener("click", () => void openExistingPath(outputPath));
element("#open-folder").addEventListener("click", () => void openExistingPath(outputPath ? parentDirectory(outputPath) : null));
element("#toast-close").addEventListener("click", () => {
  element("#toast").classList.add("hidden");
});

async function initialize(): Promise<void> {
  try {
    element("#app-version").textContent = `v${await invoke<string>("app_version")}`;
  } catch {
    element("#app-version").textContent = "version不明";
  }
  try {
    const engine = await invoke<EngineAvailability>("engine_status");
    renderEngine(engine);
    await getCurrentWebview().onDragDropEvent((event) => {
      const zone = element("#drop-zone");
      if (event.payload.type === "over") zone.classList.add("dragging");
      if (event.payload.type === "leave") zone.classList.remove("dragging");
      if (event.payload.type === "drop") {
        zone.classList.remove("dragging");
        const path = event.payload.paths.find((candidate) => candidate.toLowerCase().endsWith(".pdf"));
        if (path) void inspectPath(path);
      }
    });
  } catch (error) {
    renderEngine({ available: false, info: null, error: String(error) });
  }
}

void initialize();
