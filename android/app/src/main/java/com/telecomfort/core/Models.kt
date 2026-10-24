package com.telecomfort.core

import org.json.JSONArray
import org.json.JSONObject

data class Chat(val id: Long, val title: String, val username: String? = null,
               val unread: Int = 0, val muted: Boolean = false, val isChannel: Boolean = false,
               val archived: Boolean = false)

data class Message(val id: Int, val chatId: Long, val text: String, val date: Long,
                   val outgoing: Boolean, val sender: String?, val hasMedia: Boolean,
                   val mediaKind: String? = null, val fileName: String? = null)

data class Story(val id: Int, val peerId: Long, val caption: String, val date: Long)

data class StarTx(val id: String, val date: Long, val stars: Long,
                  val title: String?, val peer: String?, val flags: List<String>)

fun JSONObject.optStr(k: String): String? = when (val v = opt(k)) {
    is String -> v
    else -> null
}
fun JSONObject.optLong(k: String, d: Long = 0): Long = optLong(k, d)

fun parseChat(o: JSONObject) = Chat(
    id = o.optLong("id"),
    title = o.optString("title", "?"),
    username = o.optStr("username"),
    unread = o.optInt("unread_count", 0),
    muted = o.optBoolean("muted", false),
    archived = o.optBoolean("archived", false),
    isChannel = o.optBoolean("is_channel", false) || o.optLong("id") < 0,
)

fun parseMessage(o: JSONObject) = Message(
    id = o.optInt("id"),
    chatId = o.optLong("chat_id"),
    text = o.optString("text", ""),
    date = o.optLong("date"),
    outgoing = o.optBoolean("outgoing", false),
    sender = o.optStr("sender"),
    hasMedia = o.optBoolean("has_media", false),
    mediaKind = o.optStr("media_kind"),
    fileName = o.optStr("file_name"),
)

fun parseStory(o: JSONObject) = Story(
    id = o.optInt("id"),
    peerId = o.optLong("peer_id", o.optLong("chat_id")),
    caption = o.optString("caption", ""),
    date = o.optLong("date"),
)

fun JSONArray.toObjects(): List<JSONObject> = (0 until length()).map { getJSONObject(it) }
