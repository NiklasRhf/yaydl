use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_icons::Icon;
use yaydl_shared::{
    DayCount, HistoryEntry, NameCount, Statistics, StatsArgs, StatsBucket, StatsGranularity,
};

use crate::format::{self, WEEKDAYS};
use crate::ipc::{call, call0, log_to_backend};
use crate::state::use_app;
use crate::views::downloads::ConfirmButton;

const HISTORY_ROWS: usize = 200;

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Loading,
    Failed,
    Empty,
    Ready,
}

#[component]
pub fn StatisticsPage() -> impl IntoView {
    let state = use_app();
    let granularity = RwSignal::new(StatsGranularity::Day);
    let stats = RwSignal::new(None::<Statistics>);
    let loading = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let request = StoredValue::new(0u64);

    let load = move |granularity: StatsGranularity| {
        let id = request.get_value() + 1;
        request.set_value(id);
        loading.set(true);
        spawn_local(async move {
            let result = call::<_, Statistics>("get_statistics", &StatsArgs { granularity }).await;
            // A newer request superseded this one, or the page is gone.
            if request.try_get_value() != Some(id) {
                return;
            }
            loading.set(false);
            match result {
                Ok(loaded) => {
                    error.set(None);
                    stats.set(Some(loaded));
                }
                Err(e) => error.set(Some(e)),
            }
        });
    };
    Effect::new(move |_| {
        state.history_rev.track();
        load(granularity.get());
    });

    // Only a change of phase rebuilds the body, so a reload keeps the
    // segmented control and its focus.
    let phase = Memo::new(move |_| {
        stats.with(|s| match s {
            Some(s) if s.totals.downloads == 0 => Phase::Empty,
            Some(_) => Phase::Ready,
            None if error.with(Option::is_some) => Phase::Failed,
            None => Phase::Loading,
        })
    });

    view! {
        <div class="mx-auto flex max-w-5xl flex-col gap-6 p-6">
            <h1 class="text-2xl font-semibold tracking-tight">"Statistics"</h1>
            {move || {
                error
                    .get()
                    .map(|e| {
                        view! {
                            <div class="card flex flex-wrap items-center gap-3 border-red-300 p-4 dark:border-red-800">
                                <p class="min-w-0 flex-1 text-red-700 dark:text-red-400">
                                    {format!("Loading the statistics failed: {e}")}
                                </p>
                                <button class="btn btn-secondary" on:click=move |_| load(granularity.get_untracked())>
                                    "Try again"
                                </button>
                            </div>
                        }
                    })
            }}
            {move || match phase.get() {
                Phase::Loading => {
                    view! { <p class="text-zinc-500 dark:text-zinc-400">"Loading statistics\u{2026}"</p> }.into_any()
                }
                Phase::Failed => ().into_any(),
                Phase::Empty => {
                    view! {
                        <div class="card flex flex-col items-center gap-3 p-10 text-center text-zinc-500 dark:text-zinc-400">
                            <span class="text-4xl" aria-hidden="true">
                                <Icon icon=icondata::LuChartColumn />
                            </span>
                            <p class="font-medium text-zinc-700 dark:text-zinc-200">"No downloads yet"</p>
                            <p>"Finished downloads show up here with charts and your download history."</p>
                        </div>
                    }
                        .into_any()
                }
                Phase::Ready => {
                    view! { <StatsBody stats=stats granularity=granularity loading=loading /> }.into_any()
                }
            }}
            <HistorySection />
        </div>
    }
}

#[component]
fn StatsBody(
    stats: RwSignal<Option<Statistics>>,
    granularity: RwSignal<StatsGranularity>,
    loading: RwSignal<bool>,
) -> impl IntoView {
    let pick = move |f: fn(&Statistics) -> String| move || stats.with(|s| s.as_ref().map(f));
    let streak = move || {
        stats.with(|s| {
            s.as_ref().map(|s| {
                (
                    format::plural(u64::from(s.current_streak_days), "day", "days"),
                    format!(
                        "Longest {}",
                        format::plural(u64::from(s.longest_streak_days), "day", "days")
                    ),
                )
            })
        })
    };
    view! {
        <div class="grid grid-cols-2 gap-3 lg:grid-cols-4">
            <StatTile label="Total downloads" value=Signal::derive(pick(|s| format::count(u64::from(s.totals.downloads)))) />
            <StatTile label="Total size" value=Signal::derive(pick(|s| format::bytes(s.totals.bytes))) />
            <StatTile label="Total duration" value=Signal::derive(pick(|s| format::long_duration(s.totals.duration_secs))) />
            <StatTile
                label="Current streak"
                value=Signal::derive(move || streak().map(|(current, _)| current))
                note=Signal::derive(move || streak().map(|(_, longest)| longest))
            />
        </div>

        <Insights stats=stats />

        <section class="flex flex-col gap-3">
            <div class="flex flex-wrap items-center gap-3">
                <h2 class="text-lg font-semibold">"Downloads over time"</h2>
                <div class="segmented ml-auto" role="radiogroup" aria-label="Time range">
                    {StatsGranularity::ALL
                        .into_iter()
                        .map(|g| {
                            let label = match g {
                                StatsGranularity::Day => "Day",
                                StatsGranularity::Week => "Week",
                                StatsGranularity::Month => "Month",
                            };
                            view! {
                                <button
                                    role="radio"
                                    class="segmented-item"
                                    aria-checked=move || (granularity.get() == g).to_string()
                                    on:click=move |_| granularity.set(g)
                                >
                                    {label}
                                </button>
                            }
                        })
                        .collect_view()}
                </div>
            </div>
            <div
                class="card p-4 transition-opacity"
                class:opacity-60=move || loading.get()
                aria-busy=move || loading.get().to_string()
            >
                <p class="mb-2 text-sm text-zinc-500 dark:text-zinc-400">
                    {move || match granularity.get() {
                        StatsGranularity::Day => "Downloads per day, last 30 days",
                        StatsGranularity::Week => "Downloads per week, last 12 weeks",
                        StatsGranularity::Month => "Downloads per month, last 12 months",
                    }}
                </p>
                {move || {
                    stats
                        .with(|s| s.as_ref().map(|s| s.buckets.clone()))
                        .map(|buckets| view! { <BarChart buckets=buckets /> })
                }}
            </div>
        </section>

        <section class="flex flex-col gap-3">
            <h2 class="text-lg font-semibold">"When you download"</h2>
            <div class="card p-4">
                {move || {
                    stats
                        .with(|s| s.as_ref().map(|s| s.activity.clone()))
                        .map(|activity| view! { <Heatmap activity=activity /> })
                }}
            </div>
        </section>

        <div class="grid gap-3 md:grid-cols-2">
            <section class="card p-4">
                <h2 class="mb-3 font-semibold">"Top uploaders"</h2>
                {move || {
                    stats
                        .with(|s| s.as_ref().map(|s| s.top_uploaders.clone()))
                        .map(|rows| view! { <BarList rows=rows empty="No uploader information yet" /> })
                }}
            </section>
            <section class="card p-4">
                <h2 class="mb-3 font-semibold">"Formats"</h2>
                {move || {
                    stats
                        .with(|s| s.as_ref().map(|s| s.formats.clone()))
                        .map(|rows| view! { <BarList rows=rows empty="No formats yet" /> })
                }}
            </section>
        </div>
    }
}

#[component]
fn StatTile(
    label: &'static str,
    value: Signal<Option<String>>,
    #[prop(optional)] note: Option<Signal<Option<String>>>,
) -> impl IntoView {
    view! {
        <div class="card p-4">
            <p class="text-sm text-zinc-500 dark:text-zinc-400">{label}</p>
            <p class="mt-1 text-2xl font-semibold">{move || value.get()}</p>
            {note.map(|note| view! { <p class="mt-0.5 text-sm text-zinc-500 dark:text-zinc-400">{move || note.get()}</p> })}
        </div>
    }
}

fn busiest_day_sentence(day: &DayCount) -> String {
    let date = match format::iso_date(&day.date) {
        Ok(date) => date,
        Err(e) => {
            log_to_backend("warn", format!("busiest_day has an unreadable date: {e}"));
            day.date.clone()
        }
    };
    format!(
        "Busiest day: {date} with {}",
        format::plural(u64::from(day.count), "download", "downloads")
    )
}

#[component]
fn Insights(stats: RwSignal<Option<Statistics>>) -> impl IntoView {
    let sentences = move || {
        stats.with(|s| {
            let Some(s) = s else {
                return Vec::new();
            };
            let mut out = Vec::new();
            let weekday = s
                .busiest_weekday
                .and_then(|d| match WEEKDAYS.get(usize::from(d)) {
                    Some(name) => Some(*name),
                    None => {
                        log_to_backend("warn", format!("busiest_weekday is {d}, expected 0 to 6"));
                        None
                    }
                });
            match (weekday, s.busiest_hour) {
                (Some(day), Some(hour)) => {
                    out.push(format!("You're most active on {day}s around {hour:02}:00"))
                }
                (Some(day), None) => out.push(format!("You're most active on {day}s")),
                (None, Some(hour)) => out.push(format!("You're most active around {hour:02}:00")),
                (None, None) => {}
            }
            if let Some(day) = &s.busiest_day {
                out.push(busiest_day_sentence(day));
            }
            if let Some(first) = s.totals.first_download_ms {
                out.push(format!(
                    "Since {} you downloaded from {}",
                    format::date_ms(first),
                    format::plural(
                        u64::from(s.totals.distinct_uploaders),
                        "uploader",
                        "uploaders"
                    )
                ));
            }
            out
        })
    };
    view! {
        <Show when=move || !sentences().is_empty()>
            <ul class="card flex flex-col gap-1.5 p-4">
                {move || {
                    sentences()
                        .into_iter()
                        .map(|s| {
                            view! {
                                <li class="flex items-center gap-2">
                                    <span class="text-blue-600 dark:text-blue-400" aria-hidden="true">
                                        <Icon icon=icondata::LuInfo />
                                    </span>
                                    {s}
                                </li>
                            }
                        })
                        .collect_view()
                }}
            </ul>
        </Show>
    }
}

/// Upper axis bound and tick step, rounded to 1, 2 or 5 times a power of ten.
fn nice_scale(max: u32) -> (u32, u32) {
    if max == 0 {
        return (4, 1);
    }
    let raw = (f64::from(max) / 4.0).max(1.0);
    let magnitude = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * magnitude)
        .find(|s| *s >= raw)
        .unwrap_or(10.0 * magnitude) as u32;
    let top = max.div_ceil(step) * step;
    (top, step)
}

/// A column with a rounded data end and a square foot on the baseline.
fn column_path(x: f64, y: f64, w: f64, base: f64) -> String {
    let r = 4f64.min(w / 2.0).min(base - y);
    format!(
        "M{x:.2},{base:.2}V{:.2}Q{x:.2},{y:.2} {:.2},{y:.2}H{:.2}Q{:.2},{y:.2} {:.2},{:.2}V{base:.2}Z",
        y + r,
        x + r,
        x + w - r,
        x + w,
        x + w,
        y + r
    )
}

const CHART_W: f64 = 720.0;
const CHART_H: f64 = 220.0;
const MARGIN_LEFT: f64 = 40.0;
const MARGIN_RIGHT: f64 = 8.0;
const MARGIN_TOP: f64 = 10.0;
const MARGIN_BOTTOM: f64 = 26.0;

#[component]
fn BarChart(buckets: Vec<StatsBucket>) -> impl IntoView {
    let n = buckets.len().max(1) as f64;
    let plot_w = CHART_W - MARGIN_LEFT - MARGIN_RIGHT;
    let plot_h = CHART_H - MARGIN_TOP - MARGIN_BOTTOM;
    let base = MARGIN_TOP + plot_h;
    let band = plot_w / n;
    let bar_w = (band * 0.7).clamp(1.0, 24.0).min(band - 2.0).max(1.0);
    let max = buckets.iter().map(|b| b.count).max().unwrap_or(0);
    let (top, step) = nice_scale(max);
    let y_of = move |count: u32| base - f64::from(count) / f64::from(top) * plot_h;
    // Leaves at least ~56 units per label, counted back from the current
    // period so the newest bucket is always labelled.
    let label_every = (56.0 / band).ceil().max(1.0) as usize;
    let last = buckets.len().saturating_sub(1);

    let ticks = (0..=top / step)
        .map(|i| {
            let value = i * step;
            let y = y_of(value);
            view! {
                <g>
                    <line x1=MARGIN_LEFT x2=CHART_W - MARGIN_RIGHT y1=y y2=y stroke="var(--viz-grid)" stroke-width="1" />
                    <text x=MARGIN_LEFT - 8.0 y=y + 4.0 text-anchor="end" font-size="11" fill="var(--viz-label)" style="font-variant-numeric: tabular-nums">
                        {format::count(u64::from(value))}
                    </text>
                </g>
            }
        })
        .collect_view();

    let bars = buckets
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let band_x = MARGIN_LEFT + i as f64 * band;
            let x = band_x + (band - bar_w) / 2.0;
            let tip = format!(
                "{}: {}, {}",
                b.label,
                format::plural(u64::from(b.count), "download", "downloads"),
                format::bytes(b.bytes)
            );
            let column = (b.count > 0).then(|| {
                let path = column_path(x, y_of(b.count), bar_w, base);
                view! { <path class="viz-bar" d=path /> }
            });
            let label = (last - i).is_multiple_of(label_every).then(|| {
                view! {
                    <text x=band_x + band / 2.0 y=CHART_H - 8.0 text-anchor="middle" font-size="11" fill="var(--viz-label)">
                        {b.label.clone()}
                    </text>
                }
            });
            let aria = tip.clone();
            view! {
                <g class="viz-hit" tabindex="0" role="img" aria-label=aria>
                    <title>{tip}</title>
                    <rect class="viz-hit-area" x=band_x y=MARGIN_TOP width=band height=plot_h fill="transparent" rx="3" />
                    {column}
                    {label}
                </g>
            }
        })
        .collect_view();

    let rows = buckets
        .iter()
        .map(|b| {
            view! {
                <tr class="border-t border-zinc-200 dark:border-zinc-800">
                    <td class="py-1 pr-4">{b.label.clone()}</td>
                    <td class="py-1 pr-4 text-right tabular-nums">{format::count(u64::from(b.count))}</td>
                    <td class="py-1 text-right tabular-nums">{format::bytes(b.bytes)}</td>
                </tr>
            }
        })
        .collect_view();

    view! {
        <svg
            viewBox=format!("0 0 {CHART_W} {CHART_H}")
            class="h-auto w-full"
            role="group"
            aria-label="Downloads per period"
        >
            {ticks}
            <line x1=MARGIN_LEFT x2=CHART_W - MARGIN_RIGHT y1=base y2=base stroke="var(--viz-axis)" stroke-width="1" />
            {bars}
        </svg>
        <details class="mt-2 text-sm">
            <summary class="link inline-block cursor-pointer">"Show as table"</summary>
            <table class="mt-2 w-full max-w-md">
                <thead>
                    <tr class="text-left text-zinc-500 dark:text-zinc-400">
                        <th class="py-1 pr-4 font-medium">"Period"</th>
                        <th class="py-1 pr-4 text-right font-medium">"Downloads"</th>
                        <th class="py-1 text-right font-medium">"Size"</th>
                    </tr>
                </thead>
                <tbody>{rows}</tbody>
            </table>
        </details>
    }
}

const HEAT_CELL: f64 = 22.0;
const HEAT_GAP: f64 = 2.0;
const HEAT_LEFT: f64 = 40.0;
const HEAT_TOP: f64 = 18.0;
const HEAT_STEPS: u32 = 5;

fn heat_fill(value: u32, max: u32) -> String {
    if value == 0 || max == 0 {
        return "var(--viz-empty)".to_string();
    }
    let step = (f64::from(value) / f64::from(max) * f64::from(HEAT_STEPS)).ceil() as u32;
    format!("var(--seq-{})", step.clamp(1, HEAT_STEPS))
}

#[component]
fn Heatmap(activity: Vec<[u32; 24]>) -> impl IntoView {
    if activity.len() != WEEKDAYS.len() {
        let message = format!(
            "activity has {} rows, expected {}",
            activity.len(),
            WEEKDAYS.len()
        );
        log_to_backend("error", message.clone());
        return view! {
            <p class="text-sm text-red-700 dark:text-red-400">
                {format!("The activity chart cannot be drawn: {message}")}
            </p>
        }
        .into_any();
    }
    let max = activity
        .iter()
        .flat_map(|row| row.iter())
        .copied()
        .max()
        .unwrap_or(0);
    let width = HEAT_LEFT + 24.0 * HEAT_CELL;
    let height = HEAT_TOP + 7.0 * HEAT_CELL;

    let hour_labels = (0..24)
        .step_by(3)
        .map(|h| {
            view! {
                <text x=HEAT_LEFT + f64::from(h) * HEAT_CELL + (HEAT_CELL - HEAT_GAP) / 2.0 y=12 text-anchor="middle" font-size="11" fill="var(--viz-label)">
                    {format!("{h:02}")}
                </text>
            }
        })
        .collect_view();

    let rows = activity
        .iter()
        .enumerate()
        .map(|(d, row)| {
            let y = HEAT_TOP + d as f64 * HEAT_CELL;
            let cells = row
                .iter()
                .enumerate()
                .map(|(h, value)| {
                    let tip = format!(
                        "{} {h:02}:00 to {:02}:00: {}",
                        WEEKDAYS[d],
                        (h + 1) % 24,
                        format::plural(u64::from(*value), "download", "downloads")
                    );
                    view! {
                        <rect
                            class="viz-cell"
                            x=HEAT_LEFT + h as f64 * HEAT_CELL
                            y=y
                            width=HEAT_CELL - HEAT_GAP
                            height=HEAT_CELL - HEAT_GAP
                            rx="3"
                            style=format!("fill: {}", heat_fill(*value, max))
                        >
                            <title>{tip}</title>
                        </rect>
                    }
                })
                .collect_view();
            view! {
                <g>
                    <text x=HEAT_LEFT - 8.0 y=y + (HEAT_CELL - HEAT_GAP) / 2.0 + 4.0 text-anchor="end" font-size="11" fill="var(--viz-label)">
                        {&WEEKDAYS[d][..3]}
                    </text>
                    {cells}
                </g>
            }
        })
        .collect_view();

    let table_rows = activity
        .iter()
        .enumerate()
        .map(|(d, row)| {
            view! {
                <tr class="border-t border-zinc-200 dark:border-zinc-800">
                    <th class="py-1 pr-2 text-left font-medium">{&WEEKDAYS[d][..3]}</th>
                    {row
                        .iter()
                        .map(|v| view! { <td class="px-1 py-1 text-right tabular-nums">{*v}</td> })
                        .collect_view()}
                </tr>
            }
        })
        .collect_view();

    view! {
        <div class="overflow-x-auto">
            <svg
                viewBox=format!("0 0 {width} {height}")
                class="h-auto w-full min-w-[480px] max-w-3xl"
                role="img"
                aria-label="Downloads by weekday and hour"
            >
                {hour_labels}
                {rows}
            </svg>
        </div>
        <div class="mt-3 flex items-center gap-1.5 text-xs text-zinc-500 dark:text-zinc-400">
            <span>"0"</span>
            <span class="h-3 w-3 rounded-sm" style="background: var(--viz-empty)"></span>
            <span class="ml-2">"1"</span>
            {(1..=HEAT_STEPS)
                .map(|s| view! { <span class="h-3 w-3 rounded-sm" style=format!("background: var(--seq-{s})")></span> })
                .collect_view()}
            <span>{format!("{max} downloads in one hour slot")}</span>
        </div>
        <details class="mt-2 text-sm">
            <summary class="link inline-block cursor-pointer">"Show as table"</summary>
            <div class="mt-2 overflow-x-auto">
                <table class="text-xs">
                    <thead>
                        <tr class="text-zinc-500 dark:text-zinc-400">
                            <th></th>
                            {(0..24).map(|h| view! { <th class="px-1 py-1 text-right font-medium">{format!("{h:02}")}</th> }).collect_view()}
                        </tr>
                    </thead>
                    <tbody>{table_rows}</tbody>
                </table>
            </div>
        </details>
    }
    .into_any()
}

#[component]
fn BarList(rows: Vec<NameCount>, empty: &'static str) -> impl IntoView {
    if rows.is_empty() {
        return view! { <p class="text-sm text-zinc-500 dark:text-zinc-400">{empty}</p> }
            .into_any();
    }
    let max = rows.iter().map(|r| r.count).max().unwrap_or(1).max(1);
    view! {
        <ul class="flex flex-col gap-2.5">
            {rows
                .into_iter()
                .map(|row| {
                    let width = f64::from(row.count) / f64::from(max) * 100.0;
                    view! {
                        <li>
                            <div class="flex items-baseline justify-between gap-3 text-sm">
                                <span class="truncate" title=row.name.clone()>{row.name.clone()}</span>
                                <span class="shrink-0 tabular-nums text-zinc-500 dark:text-zinc-400">
                                    {format::count(u64::from(row.count))}
                                </span>
                            </div>
                            <div class="mt-1 h-2 w-full">
                                <div class="h-full rounded-r" style=format!("width: {width:.1}%; background: var(--viz-1)")></div>
                            </div>
                        </li>
                    }
                })
                .collect_view()}
        </ul>
    }
    .into_any()
}

#[component]
fn HistorySection() -> impl IntoView {
    let state = use_app();
    let toasts = state.toasts;
    let history = RwSignal::new(None::<Vec<HistoryEntry>>);
    let search = RwSignal::new(String::new());

    Effect::new(move |_| {
        state.history_rev.track();
        spawn_local(async move {
            match call0::<Vec<HistoryEntry>>("get_history").await {
                Ok(entries) => history.set(Some(entries)),
                Err(e) => toasts.error(format!("Loading the download history failed: {e}")),
            }
        });
    });

    let filtered = Memo::new(move |_| {
        let needle = search.with(|s| s.trim().to_lowercase());
        history.with(|h| {
            let Some(entries) = h else {
                return (Vec::new(), 0);
            };
            let matches: Vec<HistoryEntry> = entries
                .iter()
                .filter(|e| {
                    needle.is_empty()
                        || e.title.to_lowercase().contains(&needle)
                        || e.uploader
                            .as_ref()
                            .is_some_and(|u| u.to_lowercase().contains(&needle))
                })
                .cloned()
                .collect();
            let total = matches.len();
            (matches.into_iter().take(HISTORY_ROWS).collect(), total)
        })
    });

    let clear = Callback::new(move |()| {
        spawn_local(async move {
            if let Err(e) = call0::<()>("clear_history").await {
                toasts.error(format!("Clearing the history failed: {e}"));
            }
        });
    });

    view! {
        <section class="flex flex-col gap-3">
            <div class="flex flex-wrap items-center gap-3">
                <h2 class="text-lg font-semibold">"History"</h2>
                <label for="history-search" class="sr-only">
                    "Search history"
                </label>
                <div class="relative ml-auto">
                    <span class="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-zinc-400" aria-hidden="true">
                        <Icon icon=icondata::LuSearch />
                    </span>
                    <input
                        id="history-search"
                        type="search"
                        class="input w-64 py-1 pl-8 text-sm"
                        placeholder="Search title or uploader"
                        bind:value=search
                    />
                </div>
                <ConfirmButton
                    label="Clear history"
                    confirm_label="Click again to clear"
                    icon=icondata::LuTrash2
                    on_confirm=clear
                />
            </div>
            <div class="card overflow-x-auto">
                {move || {
                    if history.with(Option::is_none) {
                        return view! { <p class="p-4 text-sm text-zinc-500 dark:text-zinc-400">"Loading history\u{2026}"</p> }
                            .into_any();
                    }
                    let (rows, total) = filtered.get();
                    if total == 0 {
                        let text = if search.with(|s| s.trim().is_empty()) {
                            "Nothing downloaded yet"
                        } else {
                            "No downloads match the search"
                        };
                        return view! { <p class="p-4 text-sm text-zinc-500 dark:text-zinc-400">{text}</p> }.into_any();
                    }
                    let note = (total > rows.len())
                        .then(|| {
                            format!(
                                "Showing {} of {} matches. Refine the search to see others.",
                                rows.len(),
                                format::count(total as u64),
                            )
                        });
                    view! {
                        <table class="w-full text-sm">
                            <thead>
                                <tr class="text-left text-zinc-500 dark:text-zinc-400">
                                    <th class="px-4 py-2 font-medium">"Title"</th>
                                    <th class="px-4 py-2 font-medium">"Uploader"</th>
                                    <th class="px-4 py-2 font-medium">"Format"</th>
                                    <th class="px-4 py-2 font-medium">"Date"</th>
                                    <th class="px-4 py-2 text-right font-medium">"Size"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {rows
                                    .into_iter()
                                    .map(|e| {
                                        view! {
                                            <tr class="border-t border-zinc-200 dark:border-zinc-800">
                                                <td class="max-w-xs truncate px-4 py-2" title=e.title.clone()>{e.title.clone()}</td>
                                                <td class="max-w-[12rem] truncate px-4 py-2 text-zinc-600 dark:text-zinc-300">
                                                    {e.uploader.clone().unwrap_or_else(|| "Unknown".to_string())}
                                                </td>
                                                <td class="whitespace-nowrap px-4 py-2 text-zinc-600 dark:text-zinc-300">{e.format.to_string()}</td>
                                                <td class="whitespace-nowrap px-4 py-2 tabular-nums text-zinc-600 dark:text-zinc-300">
                                                    {format::date_time_ms(e.finished_at_ms)}
                                                </td>
                                                <td class="whitespace-nowrap px-4 py-2 text-right tabular-nums text-zinc-600 dark:text-zinc-300">
                                                    {e.file_size_bytes.map_or_else(|| "Unknown".to_string(), format::bytes)}
                                                </td>
                                            </tr>
                                        }
                                    })
                                    .collect_view()}
                            </tbody>
                        </table>
                        {note.map(|n| view! { <p class="border-t border-zinc-200 px-4 py-2 text-xs text-zinc-500 dark:border-zinc-800 dark:text-zinc-400">{n}</p> })}
                    }
                        .into_any()
                }}
            </div>
        </section>
    }
}
