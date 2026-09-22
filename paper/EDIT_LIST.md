# Edits to the existing paper (CTA_MSC_15.pdf) - in priority order

## A. Claims that the evidence cannot yet support (highest risk)
1. **Abstract** asserts "Evaluation ... shows improved convergence stability, reduced communication cost, and greater fairness". No evaluation exists, and the harness does not test hierarchical aggregation (the claimed cause of reduced communication). Replace with `sections/01_abstract_revised.tex`; fill `[..]` only from real results.
2. **Scope cuts.** Hierarchical aggregation, secure aggregation, differential privacy, trust/anomaly scoring, GPU-aware scoring, energy term E, "data relevance": described in Sec. III as if part of the system, but not implemented/evaluated. Add the "Prototype scope" paragraph (`02_scope_and_hypotheses.tex`) and label these *reference architecture / future work*.
3. **Intro last paragraph** promises "model accuracy, convergence time, ... scalability". Align with what is measured: round latency (p50/p95/p99), policy violations, round success, bytes, fairness, (final loss in emu/prod).
4. **Sec. III-D/F vs code.** Paper: multi-objective Eq. (1). Code: greedy top-k on a *normalised* composite score with a hard eligibility filter. Say so. In Eq. (1) state that L, T, E, C are min-max normalised to [0,1] (otherwise the weights are meaningless: ms and bytes are added to unitless terms). Add `\label{sec:selection}` to Sec. III-D (the new text refers to it).

## B. Missing required content (Exam)
5. **Sovereignty senses** (Sec. 2.1) - never mentioned. Add `02_scope_and_hypotheses.tex` (primary = legal/regulatory; secondary = operational; technical = out of scope).
6. **Uganda / DDIL context** (15 % of the grade) - the paper never mentions it. Add 1-2 paragraphs in Intro tying the setting to a Uganda-supervised federation (DPPA 2019; verify the section number; Bank of Uganda as an example supervisor if you keep it) and to rural/mobile-data links.
7. **Related-work rubric table** (`06_related_work_matrix.tex`) and **CRANE Cloud** (Bainomugisha & Mwotil, 2021) as a prior-art baseline (Exam 3.4). State your search protocol (databases, dates, inclusion criteria) in 3-4 lines - the exam asks for a *documented* protocol.
8. Experimental Methodology, Results, Limitations, Conclusion: `03..05_*.tex`.
9. **Name the target venue in writing.** The exam's own reference list contains FGCS, JPDC, IEEE TCC, SoCC, EuroSys, NSDI. If you stay with IEEE Access, note its article-processing charge (the exam cites the IEEE APC page) and confirm with the program lead.

## C. Citation numbering is broken (a reviewer will notice immediately)
The reference *list* is fine except as noted; several in-text numbers point to the wrong entry. Corrected mapping (list numbers unchanged unless stated):

| Text says | Refers to | Should be |
|---|---|---|
| "Zhang and Luo [5]" (Sec. II-B, II-C) | Zhang & Luo | **[3]** |
| "Chen et al. [6]" (II-B, II-C) | Chen et al. | **[4]** (and delete duplicate list entry [6]) |
| "Li et al. [3]" (II-A) | credit-risk FL paper - **not in your list** | add it as **[6]** (reuse the freed slot) |
| "federated semantic web ... [4]" (II-A) | **not in your list** | add as new **[22]** |
| "Lambropoulos et al. [14]" | Lambropoulos | **[17]** |
| "Marche [16]" | Marche | **[18]** |
| "Tyagi et al. [17]" | Tyagi (OmniFed) | **[16]** |
| "Shankar et al. [18]" | Shankar | **[14]** |
| "[20] federated AI orchestration in heterogeneous edge" | Atreya et al. | **[21]** |
| "[21] benchmarking study" | Errico et al. | **[20]** |
| Group cites "[1]-[4],[13],[14]" (II-A end; II-E) | financial FL works | **[1],[2],[6],[22],[13],[17]** |
| Group cite "[5]-[8],[15],[16]" (II-B end; II-E) | edge-cloud FL | **[3],[4],[7],[8],[15],[16]** (+[5] only if it is edge-cloud FL) |
| Group cite "[9]-[12],[17]-[21]" (II-E) | scheduling/orchestration | **[5],[9]-[12],[14],[16],[18]-[21]** |

Also: refs **[5],[7],[9],[10],[12]** have no authors; **[11]** (Malhotra & Baumann, "AI and Data Science Journal", no DOI) - verify it exists or remove it; **[20]** author formatting differs from the rest. I did **not** independently verify that every listed paper exists as cited; do so via each DOI. Add new refs: Bainomugisha & Mwotil (2021); Zhang & Renner (2024, artifact evaluation); Fernandez et al. (2025, arXiv:2504.03656); the Uganda DPPA 2019; tools (tonic, HdrHistogram, OPA, k3s) as software citations.

## D. Presentation defects
10. Title typo **"finacial"** (also in Index Terms).
11. Author block is the IEEE template: "(Fellow, IEEE)" (you are not), "Second B. Author", "Third C. Author, Jr.", Colorado affiliations, `10.1109/ACCESS.2017.DOI`, `VOLUME 4, 2016`, `christine.png` placeholder. Remove/replace all.
12. **Fig. 1:** the "Cloud Nodes" box is overlapped by "Institution B" - labels are illegible. **Figs. 2-3:** text is unreadable at print size; redraw at column width with >= 7 pt fonts.
13. Tables from `paper/generated/` are wide: `\genTable` uses `table*` + `\resizebox`; check they fit.

## E. How to integrate the LaTeX
```latex
% preamble (after your packages; needs booktabs, graphicx, amsmath, amssymb):
\input{sections/00_helpers}
% body:  replace abstract with 01; add 02 at the end of Sec. III; then
\input{sections/03_experimental_methodology}  \input{sections/04_results}
\input{sections/05_limitations_conclusion}    % 06 goes at the end of Related Work
```
Copy `paper/generated/` next to your main `.tex` (or symlink). The paper compiles before results exist (TODO boxes), and fills in automatically after `make analysis`.
