#pragma once

#include <nlohmann/json.hpp>

namespace sentia::apt {

class AptWorker {
 public:
  AptWorker();
  nlohmann::json Process(const nlohmann::json& request);
};

}  // namespace sentia::apt
