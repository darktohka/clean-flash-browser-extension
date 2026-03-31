package org.cleanflash.android.bridges

import org.cleanflash.android.ipc.MessageCodec
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import java.util.concurrent.TimeUnit

/**
 * HTTP request proxy — executes HTTP requests from flash-host using OkHttp.
 */
object HttpProxy {

    private val client = OkHttpClient.Builder()
        .connectTimeout(30, TimeUnit.SECONDS)
        .readTimeout(300, TimeUnit.SECONDS)
        .writeTimeout(30, TimeUnit.SECONDS)
        .followRedirects(true)
        .followSslRedirects(true)
        .build()

    /**
     * Execute an HTTP request from a binary payload and return the response.
     */
    fun executeRequest(payload: ByteArray): ByteArray {
        val reader = MessageCodec.PayloadReader(payload)
        val method = reader.readString()
        val url = reader.readString()
        val headers = reader.readString()
        val followRedirects = reader.readU8() != 0
        val hasBody = reader.readU8() != 0
        val body = if (hasBody) reader.readBytes() else null

        return try {
            val requestBuilder = Request.Builder().url(url)

            // Parse and add headers
            for (line in headers.split("\r\n")) {
                val colonIdx = line.indexOf(':')
                if (colonIdx > 0) {
                    val name = line.substring(0, colonIdx).trim()
                    val value = line.substring(colonIdx + 1).trim()
                    // Skip host header (OkHttp sets it)
                    if (!name.equals("Host", ignoreCase = true)) {
                        requestBuilder.addHeader(name, value)
                    }
                }
            }

            // Set method and body
            val contentType = headers.lines()
                .firstOrNull { it.startsWith("Content-Type:", ignoreCase = true) }
                ?.substringAfter(":")?.trim()

            val requestBody = body?.toRequestBody(contentType?.toMediaTypeOrNull())
            requestBuilder.method(method, requestBody)

            // Execute
            val httpClient = if (!followRedirects) {
                client.newBuilder()
                    .followRedirects(false)
                    .followSslRedirects(false)
                    .build()
            } else {
                client
            }

            val response = httpClient.newCall(requestBuilder.build()).execute()

            // Build response payload
            val pw = MessageCodec.PayloadWriter()
            pw.writeU16(response.code)
            pw.writeString("HTTP/${response.protocol} ${response.code} ${response.message}")

            // Response headers
            val respHeaders = StringBuilder()
            for (i in 0 until response.headers.size) {
                respHeaders.append("${response.headers.name(i)}: ${response.headers.value(i)}\r\n")
            }
            pw.writeString(respHeaders.toString())

            // Final URL (after redirects)
            val finalUrl = response.request.url.toString()
            if (finalUrl != url) {
                pw.writeU8(1)
                pw.writeString(finalUrl)
            } else {
                pw.writeU8(0)
            }

            // Body
            val bodyBytes = response.body?.bytes() ?: ByteArray(0)
            pw.writeBytes(bodyBytes)
            response.close()

            pw.finish()
        } catch (e: Exception) {
            // Return error response
            val pw = MessageCodec.PayloadWriter()
            pw.writeU16(0) // status 0 = network error
            pw.writeString("Error: ${e.message}")
            pw.writeString("")
            pw.writeU8(0)
            pw.writeBytes(ByteArray(0))
            pw.finish()
        }
    }
}
