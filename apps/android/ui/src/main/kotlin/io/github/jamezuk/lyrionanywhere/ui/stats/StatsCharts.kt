package io.github.jamezuk.lyrionanywhere.ui.stats

import android.content.res.Resources
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.data.MonthTotal
import io.github.jamezuk.lyrionanywhere.ui.R
import java.time.DayOfWeek
import java.time.YearMonth
import java.time.format.TextStyle
import java.util.Locale

/* Text alternatives for the charts: TalkBack reads these instead of the bars. */

private fun entries(res: Resources, items: List<Pair<String, Long>>): String =
    items.joinToString("; ") { (label, plays) -> res.getString(R.string.stats_chart_entry, label, playsText(res, plays)) }

/** The 24-hour chart: the busiest hour, then the next two. */
fun hourSummary(res: Resources, byHour: List<Long>): String {
    val ranked = byHour.withIndex().filter { it.value > 0 }.sortedWith(compareByDescending<IndexedValue<Long>> { it.value }.thenBy { it.index })
    val top = ranked.firstOrNull() ?: return res.getString(R.string.stats_chart_none, res.getString(R.string.stats_by_hour))
    val from = hourText(top.index)
    val to = hourText(top.index + 1)
    val next = ranked.drop(1).take(2)
    return if (next.isEmpty()) {
        res.getString(R.string.stats_chart_hours_short, from, to, playsText(res, top.value))
    } else {
        res.getString(R.string.stats_chart_hours, from, to, playsText(res, top.value), entries(res, next.map { hourText(it.index) to it.value }))
    }
}

/** "Monday" … "Sunday" (the contract's weekdays start on Monday). */
fun weekdayName(index: Int, style: TextStyle = TextStyle.FULL): String =
    DayOfWeek.of(index.mod(7) + 1).getDisplayName(style, Locale.getDefault())

/** The weekday chart: the most and the fewest, then every day. */
fun weekdaySummary(res: Resources, byWeekday: List<Long>): String {
    if (byWeekday.none { it > 0 }) return res.getString(R.string.stats_chart_none, res.getString(R.string.stats_by_weekday))
    val most = byWeekday.indices.maxBy { byWeekday[it] }
    val fewest = byWeekday.indices.minBy { byWeekday[it] }
    val all = entries(res, byWeekday.mapIndexed { i, v -> weekdayName(i) to v })
    return res.getString(R.string.stats_chart_weekday, weekdayName(most), weekdayName(fewest), all)
}

private fun monthOf(m: MonthTotal): YearMonth? = runCatching { YearMonth.parse(m.month) }.getOrNull()

private fun monthLabel(m: MonthTotal): String = monthOf(m)?.let { monthName(it.year, it.monthValue) } ?: m.month.take(16)

/** The month chart: the busiest month, then every month. */
fun monthSummary(res: Resources, byMonth: List<MonthTotal>): String {
    val top = byMonth.filter { it.plays > 0 }.maxByOrNull { it.plays }
        ?: return res.getString(R.string.stats_chart_none, res.getString(R.string.stats_by_month))
    val all = entries(res, byMonth.map { monthLabel(it) to it.plays })
    return res.getString(R.string.stats_chart_month, monthLabel(top), playsText(res, top.plays), all)
}

/**
 * A bar chart drawn on a Canvas, with labels under some bars. TalkBack reads [description]
 * (a summary with the key facts) as one item instead of the drawing.
 */
@Composable
fun BarChart(
    title: String,
    values: List<Long>,
    labels: List<String>,
    description: String,
    modifier: Modifier = Modifier,
    tag: String? = null,
) {
    val measurer = rememberTextMeasurer()
    val labelStyle = MaterialTheme.typography.labelSmall.copy(color = MaterialTheme.colorScheme.onSurfaceVariant)
    val bar = MaterialTheme.colorScheme.primary
    val peak = MaterialTheme.colorScheme.tertiary
    val track = MaterialTheme.colorScheme.surfaceVariant
    val density = LocalDensity.current
    val labelHeight = remember(measurer, labelStyle, density) {
        with(density) { measurer.measure("0", labelStyle).size.height.toDp() }
    }
    Column(modifier.padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(title, style = MaterialTheme.typography.titleSmall, modifier = Modifier.semantics { heading() })
        Canvas(
            Modifier
                .fillMaxWidth()
                .height(120.dp + labelHeight + 4.dp)
                .then(if (tag != null) Modifier.testTag(tag) else Modifier)
                .clearAndSetSemantics { contentDescription = description },
        ) {
            if (values.isEmpty()) return@Canvas
            val max = values.max().coerceAtLeast(1)
            val labelPx = labelHeight.toPx() + 4.dp.toPx()
            val chartHeight = size.height - labelPx
            val slot = size.width / values.size
            val gap = (slot * 0.2f).coerceAtMost(6.dp.toPx())
            val radius = CornerRadius(2.dp.toPx())
            for ((i, v) in values.withIndex()) {
                val x = i * slot + gap / 2
                // A faint track so empty hours still read as hours.
                drawRoundRect(track, Offset(x, chartHeight - 2.dp.toPx()), Size(slot - gap, 2.dp.toPx()), radius)
                if (v > 0) {
                    val h = (chartHeight * v / max).coerceAtLeast(2.dp.toPx())
                    drawRoundRect(if (v == max) peak else bar, Offset(x, chartHeight - h), Size(slot - gap, h), radius)
                }
                val label = labels.getOrNull(i).orEmpty()
                if (label.isNotEmpty()) {
                    val m = measurer.measure(label, labelStyle)
                    val lx = (x + (slot - gap) / 2 - m.size.width / 2).coerceIn(0f, (size.width - m.size.width).coerceAtLeast(0f))
                    drawText(m, topLeft = Offset(lx, chartHeight + 4.dp.toPx()))
                }
            }
        }
    }
}

/** Plays by hour, labelled every six hours. */
@Composable
fun HourChart(res: Resources, byHour: List<Long>) = BarChart(
    title = res.getString(R.string.stats_by_hour),
    values = byHour,
    labels = List(byHour.size) { if (it % 6 == 0) hourText(it) else "" },
    description = hourSummary(res, byHour),
    tag = "stats-chart-hour",
)

@Composable
fun WeekdayChart(res: Resources, byWeekday: List<Long>) = BarChart(
    title = res.getString(R.string.stats_by_weekday),
    values = byWeekday,
    labels = List(byWeekday.size) { weekdayName(it, TextStyle.SHORT) },
    description = weekdaySummary(res, byWeekday),
    tag = "stats-chart-weekday",
)

@Composable
fun MonthChart(res: Resources, byMonth: List<MonthTotal>) = BarChart(
    title = res.getString(R.string.stats_by_month),
    values = byMonth.map { it.plays },
    labels = byMonth.map { m -> monthOf(m)?.month?.getDisplayName(TextStyle.NARROW, Locale.getDefault()) ?: "" },
    description = monthSummary(res, byMonth),
    tag = "stats-chart-month",
)
