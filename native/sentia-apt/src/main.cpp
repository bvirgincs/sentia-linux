#include "apt_worker.h"

#include <nlohmann/json.hpp>

#include <array>
#include <iostream>
#include <string>

namespace {

constexpr size_t kMaxRequestBytes = 1024 * 1024;

std::string ReadBoundedStdin(bool& too_large, bool& read_error) {
  too_large = false;
  read_error = false;

  std::string input;
  std::array<char, 4096> buffer{};

  while (true) {
    std::cin.read(buffer.data(), static_cast<std::streamsize>(buffer.size()));
    const std::streamsize count = std::cin.gcount();

    if (count > 0) {
      const size_t as_size = static_cast<size_t>(count);
      if (input.size() + as_size > kMaxRequestBytes) {
        too_large = true;
        break;
      }
      input.append(buffer.data(), as_size);
    }

    if (std::cin.eof()) {
      break;
    }

    if (std::cin.bad()) {
      read_error = true;
      break;
    }

    if (count == 0) {
      break;
    }
  }

  return input;
}

nlohmann::json BaseErrorResponse(const std::string& code, const std::string& message,
                                 nlohmann::json details = nlohmann::json::object()) {
  nlohmann::json response = {
      {"request_id", nullptr},
      {"protocol_version", "1.0"},
      {"operation", nullptr},
      {"status", "error"},
      {"error", {{"code", code}, {"message", message}}}};
  if (!details.empty()) {
    response["error"]["details"] = details;
  }
  return response;
}

}  // namespace

int main() {
  std::ios::sync_with_stdio(false);
  std::cin.tie(nullptr);

  bool request_too_large = false;
  bool stdin_read_error = false;
  const std::string input = ReadBoundedStdin(request_too_large, stdin_read_error);

  if (request_too_large) {
    const nlohmann::json response =
        BaseErrorResponse("request_too_large", "Request body exceeds maximum size",
                          {{"max_request_bytes", kMaxRequestBytes}});
    std::cout << response.dump() << '\n';
    return 65;
  }

  if (stdin_read_error) {
    const nlohmann::json response = BaseErrorResponse(
        "stdin_read_error", "Failed while reading request body from stdin");
    std::cout << response.dump() << '\n';
    return 66;
  }

  nlohmann::json parsed;
  try {
    parsed = nlohmann::json::parse(input.empty() ? "{}" : input);
  } catch (const std::exception& error) {
    const nlohmann::json response =
        BaseErrorResponse("invalid_json", "Request body is not valid JSON",
                          {{"what", error.what()}});
    std::cout << response.dump() << '\n';
    return 64;
  }

  sentia::apt::AptWorker worker;
  nlohmann::json response = worker.Process(parsed);
  std::cout << response.dump() << '\n';
  return response.value("status", "error") == "ok" ? 0 : 2;
}
