package com.telecomfort.ui.screens

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.dp
import org.json.JSONArray
import org.json.JSONObject

/** Мини-график временного ряда: сетка + ломаная + мин/макс (зеркало десктопного draw_series). */
@Composable
fun MiniChart(labels: List<String>, values: List<Double>, modifier: Modifier = Modifier) {
    val measurer = rememberTextMeasurer()
    val line = MaterialTheme.colorScheme.primary
    val grid = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.15f)
    if (values.size < 2) {
        Text("мало точек", style = MaterialTheme.typography.bodySmall)
        return
    }
    val min = values.minOrNull() ?: 0.0
    val max = values.maxOrNull() ?: 1.0
    val span = (max - min).takeIf { it > 0 } ?: 1.0
    Canvas(modifier.height(140.dp).fillMaxWidth()) {
        val padL = 44f; val padT = 8f; val padB = 22f; val padR = 8f
        val w = size.width - padL - padR
        val h = size.height - padT - padB
        // Сетка.
        for (i in 0..2) {
            val y = padT + h * i / 2
            drawLine(grid, Offset(padL, y), Offset(padL + w, y), strokeWidth = 1f)
            val v = max - span * i / 2
            drawText(measurer, "%.0f".format(v), topLeft = Offset(2f, y - 18f))
        }
        // Ломаная.
        val pts = values.mapIndexed { i, v ->
            Offset(
                padL + w * i / (values.size - 1).coerceAtLeast(1).toFloat(),
                (padT + h * (1 - (v - min) / span)).toFloat(),
            )
        }
        val path = Path().apply {
            moveTo(pts[0].x, pts[0].y)
            pts.drop(1).forEach { lineTo(it.x, it.y) }
        }
        drawPath(path, line, style = Stroke(2.5f))
        drawCircle(line, 3.5f, pts.last())
        // Подписи крайних дат.
        if (labels.isNotEmpty()) {
            drawText(measurer, labels.first().take(10), topLeft = Offset(padL, size.height - 20))
            drawText(measurer, labels.last().take(10), topLeft = Offset(size.width - 120f, size.height - 20))
        }
    }
}

/** Парсинг channel_graph JSON: {labels:[...], values:[...]} */
fun parseSeries(o: JSONObject): Pair<List<String>, List<Double>> {
    val la = o.optJSONArray("labels") ?: JSONArray()
    val va = o.optJSONArray("values") ?: JSONArray()
    val labels = (0 until la.length()).map { la.getString(it) }
    val values = (0 until va.length()).map { va.getDouble(it) }
    return labels to values
}
