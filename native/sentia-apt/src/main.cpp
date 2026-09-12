#include "apt_worker.h"

#include <nlohmann/json.hpp>

#include <iostream>
#include <iterator>
#include <string>

int main() {
  std::ios::sync_with_stdio(false);
  std::cin.tie(nullptr);

  const std::string input((std::istreambuf_iterator<char>(std::cin)),
                          std::istreambuf_iterator<char>());

  nlohmann::json parsed;
  try {
    parsed = nlohmann::json::parse(input.empty() ? "{}" : input);
  } catch (const std::exception& error) {
    nlohmann::json response = {
        {"request_id", nullptr},
        {"protocol_version", "1.0"},
        {"operation", nullptr},
        {"status", "error"},
        {"error",
         {{"code", "invalid_json"},
          {"message", "Request body is not valid JSON"},
          {"details", {{"what", error.what()}}}}}};
    std::cout << response.dump() << '\n';
    return 64;
  }

  sentia::apt::AptWorker worker;
  nlohmann::json response = worker.Process(parsed);
  std::cout << response.dump() << '\n';
  return response.value("status", "error") == "ok" ? 0 : 2;
}
