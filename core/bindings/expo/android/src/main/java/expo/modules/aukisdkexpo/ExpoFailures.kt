package expo.modules.aukisdkexpo

import expo.modules.kotlin.exception.CodedException
import uniffi.auki_sdk_uniffi.AukiAuthFailureKind
import uniffi.auki_sdk_uniffi.AukiDataFailureKind
import uniffi.auki_sdk_uniffi.AukiJobsFailureKind
import uniffi.auki_sdk_uniffi.AukiSdkException

internal class ExpoUnsupported(message: String) :
  CodedException("AukiSdkExpoUnsupported", message, null)

internal class ExpoAuthFailure(kind: AukiAuthFailureKind) :
  CodedException(expoAuthCode(kind), "Auki authentication: ${expoAuthCode(kind)}", null)

internal class ExpoDataFailure(
  kind: String,
  status: Int?,
  authCode: String,
  message: String,
) : CodedException(
  listOf("domain_data", kind, status?.toString() ?: "", authCode).joinToString(":"),
  message,
  null,
)

internal class ExpoJobsFailure(
  kind: String,
  status: Int?,
  code: String?,
  maximum: ULong?,
  source: String?,
  retryAfterSeconds: UInt?,
  message: String,
) : CodedException(
  listOf(
    "jobs",
    kind,
    status?.toString() ?: "",
    if (kind == "auth") code ?: "" else "",
    source ?: "",
    retryAfterSeconds?.toString() ?: "",
  ).joinToString(":"),
  if (maximum == null) message else "$message (maximum: $maximum bytes)",
  null,
)

internal class ExpoFleetFailure(
  kind: String,
  status: Int?,
  code: String,
  message: String,
) : CodedException(
  listOf("fleet", kind, status?.toString() ?: "", code).joinToString(":"),
  message,
  null,
)

internal fun expoAuthCode(kind: AukiAuthFailureKind): String = when (kind) {
  AukiAuthFailureKind.AUTHENTICATION_REQUIRED -> "authentication_required"
  AukiAuthFailureKind.CONFIGURATION -> "configuration"
  AukiAuthFailureKind.AUTHORIZATION_DENIED -> "authorization_denied"
  AukiAuthFailureKind.PERSISTENCE -> "persistence"
  AukiAuthFailureKind.TRANSIENT -> "transient"
  AukiAuthFailureKind.CANCELLED -> "cancelled"
  AukiAuthFailureKind.CLOSED -> "closed"
}

internal fun expoDataFailureKind(kind: AukiDataFailureKind): String = when (kind) {
  AukiDataFailureKind.AUTHENTICATION -> "auth"
  AukiDataFailureKind.HTTP -> "http"
  AukiDataFailureKind.INVALID_INPUT -> "input"
  AukiDataFailureKind.INVALID_RESPONSE -> "response"
  AukiDataFailureKind.LIMIT -> "limit"
  AukiDataFailureKind.CANCELLED -> "cancelled"
  AukiDataFailureKind.CLOSED -> "closed"
  AukiDataFailureKind.TIMEOUT -> "timeout"
  AukiDataFailureKind.TRANSPORT -> "transport"
  AukiDataFailureKind.CALLBACK -> "callback"
  AukiDataFailureKind.CLEANUP -> "cleanup"
}

internal fun expoJobsFailureKind(kind: AukiJobsFailureKind): String = when (kind) {
  AukiJobsFailureKind.AUTHENTICATION -> "auth"
  AukiJobsFailureKind.INVALID_INPUT -> "invalid_input"
  AukiJobsFailureKind.INVALID_RESPONSE -> "invalid_response"
  AukiJobsFailureKind.HTTP_STATUS -> "http_status"
  AukiJobsFailureKind.TRANSPORT -> "transport"
  AukiJobsFailureKind.TIMED_OUT -> "timed_out"
  AukiJobsFailureKind.CANCELLED -> "cancelled"
  AukiJobsFailureKind.CLOSED -> "closed"
  AukiJobsFailureKind.TOO_LARGE -> "too_large"
  AukiJobsFailureKind.SUBMISSION_UNCERTAIN -> "submission_uncertain"
  AukiJobsFailureKind.SUBMISSION_IN_PROGRESS -> "submission_in_progress"
}

internal suspend fun <T> withAuthErrors(block: suspend () -> T): T {
  try {
    return block()
  } catch (error: AukiSdkException.Authentication) {
    throw ExpoAuthFailure(error.kind)
  }
}

internal suspend fun <T> withDataErrors(block: suspend () -> T): T {
  try {
    return block()
  } catch (error: AukiSdkException.DomainData) {
    throw ExpoDataFailure(
      kind = expoDataFailureKind(error.kind),
      status = error.status?.toInt(),
      authCode = error.authKind?.let(::expoAuthCode) ?: "",
      message = error.errorMessage,
    )
  }
}

internal suspend fun <T> withJobsErrors(block: suspend () -> T): T {
  try {
    return block()
  } catch (error: AukiSdkException.Jobs) {
    throw ExpoJobsFailure(
      kind = expoJobsFailureKind(error.kind),
      status = error.status?.toInt(),
      code = error.code,
      maximum = error.maximum,
      source = error.sourceCode,
      retryAfterSeconds = error.retryAfterSeconds,
      message = error.errorMessage,
    )
  }
}

internal suspend fun <T> withFleetErrors(block: suspend () -> T): T {
  try {
    return block()
  } catch (error: AukiSdkException.Fleet) {
    throw ExpoFleetFailure(
      kind = error.kind,
      status = error.status?.toInt(),
      code = error.code,
      message = error.errorMessage,
    )
  }
}
