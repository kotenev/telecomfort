package com.telecomfort.core

/** Видимость Activity: сервис не дублирует пуш, когда чат открыт. */
object AppVisibility {
    @Volatile
    var visible: Boolean = false
}
