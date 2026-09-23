use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_icons::Icon;
use yaydl_shared::{
    DayCount, HistoryEntry, NameCount, Statistics, StatsArgs, StatsBucket, StatsGranularity,
};

use crate::format;
use crate::i18n::{format_label, format_name_label, texts_now, use_texts, Text, Texts};
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
    let t = use_texts();
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
            <h1 class="text-2xl font-semibold tracking-tight">{move || t().stats_title}</h1>
            {move || {
                error
                    .get()
                    .map(|e| {
                        view! {
                            <div class="card flex flex-wrap items-center gap-3 border-red-300 p-4 dark:border-red-800">
                                <p class="min-w-0 flex-1 text-red-700 dark:text-red-400">
                                    {move || (t().stats_load_failed)(&e)}
                                </p>
                                <button class="btn btn-secondary" on:click=move |_| load(granularity.get_untracked())>
                                    {move || t().try_again}
                                </button>
                            </div>
                        }
                    })
            }}
            {move || match phase.get() {
                Phase::Loading => {
                    view! { <p class="text-zinc-500 dark:text-zinc-400">{move || t().stats_loading}</p> }.into_any()
                }
                Phase::Failed => ().into_any(),
                Phase::Empty => {
                    view! {
                        <div class="card flex flex-col items-center gap-3 p-10 text-center text-zinc-500 dark:text-zinc-400">
                            <span class="text-4xl" aria-hidden="true">
                                <Icon icon=icondata::LuChartColumn />
                            </span>
                            <p class="font-medium text-zinc-700 dark:text-zinc-200">{move || t().stats_empty_title}</p>
                            <p>{move || t().stats_empty_body}</p>
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
    let t = use_texts();
    let pick = move |f: fn(&Statistics, &Texts) -> String| {
        move || {
            let t = t();
            stats.with(|s| s.as_ref().map(|s| f(s, t)))
        }
    };
    view! {
        <div class="grid grid-cols-2 gap-3 lg:grid-cols-4">
            <StatTile
                label=|t| t.tile_total_downloads
                value=Signal::derive(pick(|s, t| format::count(u64::from(s.totals.downloads), t.locale)))
            />
            <StatTile
                label=|t| t.tile_total_size
                value=Signal::derive(pick(|s, t| format::bytes(s.totals.bytes, t.locale)))
            />
            <StatTile
                label=|t| t.tile_total_duration
                value=Signal::derive(pick(|s, t| format::long_duration(s.totals.duration_secs, t.locale)))
            />
            <StatTile
                label=|t| t.tile_streak
                value=Signal::derive(pick(|s, t| (t.days)(u64::from(s.current_streak_days))))
                note=Signal::derive(pick(|s, t| (t.streak_longest)(&(t.days)(u64::from(s.longest_streak_days)))))
            />
        </div>

        <Insights stats=stats />

        <section class="flex flex-col gap-3">
            <div class="flex flex-wrap items-center gap-3">
                <h2 class="text-lg font-semibold">{move || t().over_time}</h2>
                <div class="segmented ml-auto" role="radiogroup" aria-label=move || t().time_range>
                    {StatsGranularity::ALL
                        .into_iter()
                        .map(|g| {
                            let label = move || match g {
                                StatsGranularity::Day => t().granularity_day,
                                StatsGranularity::Week => t().granularity_week,
                                StatsGranularity::Month => t().granularity_month,
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
                        StatsGranularity::Day => t().caption_day,
                        StatsGranularity::Week => t().caption_week,
                        StatsGranularity::Month => t().caption_month,
                    }}
                </p>
                {move || {
                    let t = t();
                    stats
                        .with(|s| s.as_ref().map(|s| (s.buckets.clone(), s.granularity)))
                        .map(|(buckets, granularity)| {
                            view! { <BarChart buckets=buckets granularity=granularity t=t /> }
                        })
                }}
            </div>
        </section>

        <section class="flex flex-col gap-3">
            <h2 class="text-lg font-semibold">{move || t().when_you_download}</h2>
            <div class="card p-4">
                {move || {
                    let t = t();
                    stats
                        .with(|s| s.as_ref().map(|s| s.activity.clone()))
                        .map(|activity| view! { <Heatmap activity=activity t=t /> })
                }}
            </div>
        </section>

        <div class="grid gap-3 md:grid-cols-2">
            <section class="card p-4">
                <h2 class="mb-3 font-semibold">{move || t().top_uploaders}</h2>
                {move || {
                    let t = t();
                    stats
                        .with(|s| s.as_ref().map(|s| s.top_uploaders.clone()))
                        .map(|rows| view! { <BarList rows=rows empty=t.no_uploaders t=t /> })
                }}
            </section>
            <section class="card p-4">
                <h2 class="mb-3 font-semibold">{move || t().formats}</h2>
                {move || {
                    let t = t();
                    stats
                        .with(|s| {
                            s.as_ref()
                                .map(|s| {
                                    s.formats
                                        .iter()
                                        .map(|row| NameCount {
                                            name: format_name_label(t, &row.name),
                                            count: row.count,
                                        })
                                        .collect::<Vec<_>>()
                                })
                        })
                        .map(|rows| view! { <BarList rows=rows empty=t.no_formats t=t /> })
                }}
            </section>
        </div>
    }
}

#[component]
fn StatTile(
    label: Text,
    value: Signal<Option<String>>,
    #[prop(optional)] note: Option<Signal<Option<String>>>,
) -> impl IntoView {
    let t = use_texts();
    view! {
        <div class="card p-4">
            <p class="text-sm text-zinc-500 dark:text-zinc-400">{move || label(t())}</p>
            <p class="mt-1 text-2xl font-semibold">{move || value.get()}</p>
            {note.map(|note| view! { <p class="mt-0.5 text-sm text-zinc-500 dark:text-zinc-400">{move || note.get()}</p> })}
        </div>
    }
}

fn busiest_day_sentence(t: &Texts, day: &DayCount) -> String {
    let date = match format::iso_date(&day.date, t.locale) {
        Ok(date) => date,
        Err(e) => {
            log_to_backend("warn", format!("busiest_day has an unreadable date: {e}"));
            day.date.clone()
        }
    };
    (t.busiest_day)(&date, u64::from(day.count))
}

fn insight_sentences(t: &Texts, s: &Statistics) -> Vec<String> {
    let mut out = Vec::new();
    let weekday = s
        .busiest_weekday
        .and_then(|d| match t.weekdays_habitual.get(usize::from(d)) {
            Some(name) => Some(*name),
            None => {
                log_to_backend("warn", format!("busiest_weekday is {d}, expected 0 to 6"));
                None
            }
        });
    let hour = s.busiest_hour.map(|h| format::around_hour(h, t.locale));
    match (weekday, hour) {
        (Some(day), Some(hour)) => out.push((t.most_active_day_hour)(day, &hour)),
        (Some(day), None) => out.push((t.most_active_day)(day)),
        (None, Some(hour)) => out.push((t.most_active_hour)(&hour)),
        (None, None) => {}
    }
    if let Some(day) = &s.busiest_day {
        out.push(busiest_day_sentence(t, day));
    }
    if let Some(first) = s.totals.first_download_ms {
        out.push((t.since_uploaders)(
            &format::date_ms(first, t.locale),
            u64::from(s.totals.distinct_uploaders),
        ));
    }
    out
}

#[component]
fn Insights(stats: RwSignal<Option<Statistics>>) -> impl IntoView {
    let t = use_texts();
    let sentences = move || {
        let t = t();
        stats.with(|s| {
            s.as_ref()
                .map_or_else(Vec::new, |s| insight_sentences(t, s))
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

fn bucket_label(t: &Texts, bucket: &StatsBucket, granularity: StatsGranularity) -> String {
    match format::bucket_label(&bucket.start_date, granularity, t.locale) {
        Ok(label) => label,
        Err(e) => {
            log_to_backend(
                "warn",
                format!("a statistics bucket has an unreadable start date: {e}"),
            );
            bucket.start_date.clone()
        }
    }
}

const CHART_W: f64 = 720.0;
const CHART_H: f64 = 220.0;
const MARGIN_LEFT: f64 = 40.0;
const MARGIN_RIGHT: f64 = 8.0;
const MARGIN_TOP: f64 = 10.0;
const MARGIN_BOTTOM: f64 = 26.0;

#[component]
fn BarChart(
    buckets: Vec<StatsBucket>,
    granularity: StatsGranularity,
    t: &'static Texts,
) -> impl IntoView {
    let locale = t.locale;
    let labels: Vec<String> = buckets
        .iter()
        .map(|b| bucket_label(t, b, granularity))
        .collect();
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
                        {format::count(u64::from(value), locale)}
                    </text>
                </g>
            }
        })
        .collect_view();

    let bars = buckets
        .iter()
        .zip(&labels)
        .enumerate()
        .map(|(i, (b, label))| {
            let band_x = MARGIN_LEFT + i as f64 * band;
            let x = band_x + (band - bar_w) / 2.0;
            let tip = (t.bar_tip)(label, u64::from(b.count), &format::bytes(b.bytes, locale));
            let column = (b.count > 0).then(|| {
                let path = column_path(x, y_of(b.count), bar_w, base);
                view! { <path class="viz-bar" d=path /> }
            });
            // The newest label sits at the right edge, centering it would clip it.
            let (label_x, anchor) = if i == last {
                (band_x + band, "end")
            } else {
                (band_x + band / 2.0, "middle")
            };
            let axis_label = (last - i).is_multiple_of(label_every).then(|| {
                view! {
                    <text x=label_x y=CHART_H - 8.0 text-anchor=anchor font-size="11" fill="var(--viz-label)">
                        {label.clone()}
                    </text>
                }
            });
            let aria = tip.clone();
            view! {
                <g class="viz-hit" tabindex="0" role="img" aria-label=aria>
                    <title>{tip}</title>
                    <rect class="viz-hit-area" x=band_x y=MARGIN_TOP width=band height=plot_h fill="transparent" rx="3" />
                    {column}
                    {axis_label}
                </g>
            }
        })
        .collect_view();

    let rows = buckets
        .iter()
        .zip(labels)
        .map(|(b, label)| {
            view! {
                <tr class="border-t border-zinc-200 dark:border-zinc-800">
                    <td class="py-1 pr-4">{label}</td>
                    <td class="py-1 pr-4 text-right tabular-nums">{format::count(u64::from(b.count), locale)}</td>
                    <td class="py-1 text-right tabular-nums">{format::bytes(b.bytes, locale)}</td>
                </tr>
            }
        })
        .collect_view();

    view! {
        <svg
            viewBox=format!("0 0 {CHART_W} {CHART_H}")
            class="h-auto w-full"
            role="group"
            aria-label=t.chart_label
        >
            {ticks}
            <line x1=MARGIN_LEFT x2=CHART_W - MARGIN_RIGHT y1=base y2=base stroke="var(--viz-axis)" stroke-width="1" />
            {bars}
        </svg>
        <details class="mt-2 text-sm">
            <summary class="link inline-block cursor-pointer">{t.show_table}</summary>
            <table class="mt-2 w-full max-w-md">
                <thead>
                    <tr class="text-left text-zinc-500 dark:text-zinc-400">
                        <th class="py-1 pr-4 font-medium">{t.period}</th>
                        <th class="py-1 pr-4 text-right font-medium">{t.col_downloads}</th>
                        <th class="py-1 text-right font-medium">{t.size}</th>
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
fn Heatmap(activity: Vec<[u32; 24]>, t: &'static Texts) -> impl IntoView {
    if activity.len() != t.weekdays.len() {
        let message = format!(
            "activity has {} rows, expected {}",
            activity.len(),
            t.weekdays.len()
        );
        log_to_backend("error", message.clone());
        return view! {
            <p class="text-sm text-red-700 dark:text-red-400">
                {(t.heatmap_broken)(&message)}
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
                    let tip = (t.heat_tip)(t.weekdays[d], h, u64::from(*value));
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
                        {t.weekdays_short[d]}
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
                    <th class="py-1 pr-2 text-left font-medium">{t.weekdays_short[d]}</th>
                    {row
                        .iter()
                        .map(|v| view! { <td class="px-1 py-1 text-right tabular-nums">{format::count(u64::from(*v), t.locale)}</td> })
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
                aria-label=t.heatmap_label
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
            <span>{(t.heat_legend)(u64::from(max))}</span>
        </div>
        <details class="mt-2 text-sm">
            <summary class="link inline-block cursor-pointer">{t.show_table}</summary>
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
fn BarList(rows: Vec<NameCount>, empty: &'static str, t: &'static Texts) -> impl IntoView {
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
                                    {format::count(u64::from(row.count), t.locale)}
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

/// The file name on disk, which differs from the video title after a rename.
fn history_name(entry: &HistoryEntry) -> String {
    let path = entry.file_path.to_string_lossy();
    match format::file_stem(&path) {
        Some(stem) => stem.to_string(),
        None => {
            log_to_backend(
                "warn",
                format!(
                    "history entry {} has a file path without a file name: {path:?}",
                    entry.download_id
                ),
            );
            entry.title.clone()
        }
    }
}

/// The video title, unless the file name already says it. yt-dlp's default
/// name is `<title> [<id>]`, which counts as saying it.
fn history_subtitle(entry: &HistoryEntry, name: &str) -> Option<String> {
    let default_name = format!("{} [{}]", entry.title, entry.video_id);
    (name != entry.title && name != default_name).then(|| entry.title.clone())
}

#[component]
fn HistorySection() -> impl IntoView {
    let state = use_app();
    let toasts = state.toasts;
    let t = use_texts();
    let history = RwSignal::new(None::<Vec<HistoryEntry>>);
    let search = RwSignal::new(String::new());

    Effect::new(move |_| {
        state.history_rev.track();
        spawn_local(async move {
            match call0::<Vec<HistoryEntry>>("get_history").await {
                Ok(entries) => history.set(Some(entries)),
                Err(e) => toasts.error((texts_now(state).history_load_failed)(&e)),
            }
        });
    });

    let filtered = Memo::new(move |_| {
        let needle = search.with(|s| s.trim().to_lowercase());
        history.with(|h| {
            let Some(entries) = h else {
                return (Vec::new(), 0);
            };
            let matches: Vec<(HistoryEntry, String)> = entries
                .iter()
                .map(|e| (e, history_name(e)))
                .filter(|(e, name)| {
                    needle.is_empty()
                        || name.to_lowercase().contains(&needle)
                        || e.title.to_lowercase().contains(&needle)
                        || e.uploader
                            .as_ref()
                            .is_some_and(|u| u.to_lowercase().contains(&needle))
                })
                .map(|(e, name)| (e.clone(), name))
                .collect();
            let total = matches.len();
            (matches.into_iter().take(HISTORY_ROWS).collect(), total)
        })
    });

    let clear = Callback::new(move |()| {
        spawn_local(async move {
            if let Err(e) = call0::<()>("clear_history").await {
                toasts.error((texts_now(state).history_clear_failed)(&e));
            }
        });
    });

    view! {
        <section class="flex flex-col gap-3">
            <div class="flex flex-wrap items-center gap-3">
                <h2 class="text-lg font-semibold">{move || t().history_title}</h2>
                <label for="history-search" class="sr-only">
                    {move || t().history_search_label}
                </label>
                <div class="relative ml-auto">
                    <span class="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-zinc-400" aria-hidden="true">
                        <Icon icon=icondata::LuSearch />
                    </span>
                    <input
                        id="history-search"
                        type="search"
                        class="input w-64 py-1 pl-8 text-sm"
                        placeholder=move || t().history_search_placeholder
                        bind:value=search
                    />
                </div>
                <ConfirmButton
                    label=|t| t.clear_history
                    confirm_label=|t| t.clear_history_confirm
                    icon=icondata::LuTrash2
                    on_confirm=clear
                />
            </div>
            <div class="card overflow-x-auto">
                {move || {
                    let t = t();
                    let locale = t.locale;
                    if history.with(Option::is_none) {
                        return view! { <p class="p-4 text-sm text-zinc-500 dark:text-zinc-400">{t.history_loading}</p> }
                            .into_any();
                    }
                    let (rows, total) = filtered.get();
                    if total == 0 {
                        let text = if search.with(|s| s.trim().is_empty()) {
                            t.history_empty
                        } else {
                            t.history_no_match
                        };
                        return view! { <p class="p-4 text-sm text-zinc-500 dark:text-zinc-400">{text}</p> }.into_any();
                    }
                    let note = (total > rows.len()).then(|| (t.history_truncated)(rows.len(), total));
                    view! {
                        <table class="w-full text-sm">
                            <thead>
                                <tr class="text-left text-zinc-500 dark:text-zinc-400">
                                    <th class="px-4 py-2 font-medium">{t.col_name}</th>
                                    <th class="px-4 py-2 font-medium">{t.col_uploader}</th>
                                    <th class="px-4 py-2 font-medium">{t.col_format}</th>
                                    <th class="px-4 py-2 font-medium">{t.col_date}</th>
                                    <th class="px-4 py-2 text-right font-medium">{t.size}</th>
                                </tr>
                            </thead>
                            <tbody>
                                {rows
                                    .into_iter()
                                    .map(|(e, name)| {
                                        let subtitle = history_subtitle(&e, &name);
                                        let name_tip = name.clone();
                                        view! {
                                            <tr class="border-t border-zinc-200 dark:border-zinc-800">
                                                <td class="max-w-xs px-4 py-2">
                                                    <p class="truncate" title=name_tip>{name}</p>
                                                    {subtitle
                                                        .map(|s| {
                                                            let tip = s.clone();
                                                            view! {
                                                                <p class="truncate text-xs text-zinc-500 dark:text-zinc-400" title=tip>
                                                                    {s}
                                                                </p>
                                                            }
                                                        })}
                                                </td>
                                                <td class="max-w-[12rem] truncate px-4 py-2 text-zinc-600 dark:text-zinc-300">
                                                    {e.uploader.clone().unwrap_or_else(|| t.unknown.to_string())}
                                                </td>
                                                <td class="whitespace-nowrap px-4 py-2 text-zinc-600 dark:text-zinc-300">{format_label(t, e.format)}</td>
                                                <td class="whitespace-nowrap px-4 py-2 tabular-nums text-zinc-600 dark:text-zinc-300">
                                                    {format::date_time_ms(e.finished_at_ms, locale)}
                                                </td>
                                                <td class="whitespace-nowrap px-4 py-2 text-right tabular-nums text-zinc-600 dark:text-zinc-300">
                                                    {e.file_size_bytes.map_or_else(|| t.unknown.to_string(), |b| format::bytes(b, locale))}
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

#[cfg(test)]
mod tests {
    use super::*;
    use yaydl_shared::{AudioCodec, OutputFormat};

    fn entry(title: &str, path: &str) -> HistoryEntry {
        HistoryEntry {
            download_id: 1,
            extractor: "Youtube".to_string(),
            video_id: "abc123".to_string(),
            title: title.to_string(),
            uploader: None,
            url: "https://youtu.be/abc123".to_string(),
            format: OutputFormat::Audio {
                codec: AudioCodec::Mp3,
            },
            file_path: path.into(),
            file_size_bytes: None,
            duration_secs: None,
            finished_at_ms: 0,
        }
    }

    #[test]
    fn renamed_history_entries_show_the_file_name() {
        let renamed = entry("DUNE: Part Three | War Chant Lyrics", "/m/War Chant.mp3");
        let name = history_name(&renamed);
        assert_eq!(name, "War Chant");
        assert_eq!(
            history_subtitle(&renamed, &name).as_deref(),
            Some("DUNE: Part Three | War Chant Lyrics")
        );

        let default = entry("Song", "/m/Song [abc123].mp3");
        let name = history_name(&default);
        assert_eq!(name, "Song [abc123]");
        assert_eq!(history_subtitle(&default, &name), None);
    }

    #[test]
    fn insights_are_localized() {
        let stats = Statistics {
            granularity: StatsGranularity::Day,
            buckets: Vec::new(),
            totals: Default::default(),
            activity: vec![[0; 24]; 7],
            busiest_weekday: Some(6),
            busiest_hour: Some(21),
            busiest_day: Some(DayCount {
                date: "2026-09-12".to_string(),
                count: 1,
            }),
            current_streak_days: 0,
            longest_streak_days: 0,
            top_uploaders: Vec::new(),
            formats: Vec::new(),
        };
        assert_eq!(
            insight_sentences(&crate::i18n::DE, &stats),
            [
                "Am aktivsten bist du sonntags gegen 21 Uhr",
                "Aktivster Tag: 12. Sep. 2026 mit 1 Download",
            ]
        );
        assert_eq!(
            insight_sentences(&crate::i18n::EN, &stats),
            [
                "You're most active on Sundays around 21:00",
                "Busiest day: 12 Sep 2026 with 1 download",
            ]
        );
    }
}
