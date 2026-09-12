#include "apt_worker.h"

#include "sha256.h"

#include <apt-pkg/acquire.h>
#include <apt-pkg/algorithms.h>
#include <apt-pkg/cachefile.h>
#include <apt-pkg/configuration.h>
#include <apt-pkg/depcache.h>
#include <apt-pkg/error.h>
#include <apt-pkg/init.h>
#include <apt-pkg/install-progress.h>
#include <apt-pkg/packagemanager.h>
#include <apt-pkg/pkgcache.h>
#include <apt-pkg/policy.h>
#include <apt-pkg/pkgrecords.h>
#include <apt-pkg/pkgsystem.h>
#include <apt-pkg/sourcelist.h>
#include <apt-pkg/update.h>
#include <apt-pkg/upgrade.h>

#include <nlohmann/json.hpp>

#include <algorithm>
#include <chrono>
#include <cctype>
#include <cstdlib>
#include <ctime>
#include <filesystem>
#include <fstream>
#include <iomanip>
#include <memory>
#include <optional>
#include <regex>
#include <set>
#include <sstream>
#include <stdexcept>
#include <string>
#include <unordered_set>
#include <utility>
#include <vector>

#include <unistd.h>

namespace sentia::apt {
namespace {

using json = nlohmann::json;

constexpr char kProtocolVersion[] = "1.0";
constexpr size_t kMaxRequestIdLength = 128;
constexpr size_t kMaxPackageCount = 64;
constexpr size_t kMaxSearchQueryLength = 256;
constexpr int kMaxSearchLimit = 200;
constexpr int kDefaultSearchLimit = 25;
constexpr int kDefaultVersionLimit = 10;
constexpr int kMaxVersionLimit = 50;
constexpr int kMaxDependencyEntries = 512;
constexpr size_t kMaxFilePathLength = 4096;
constexpr size_t kMaxCommandIndexSizeBytes = 32ULL * 1024ULL * 1024ULL;
constexpr char kPackageNameRegex[] = "^[a-z0-9][a-z0-9+.-]{0,127}$";
constexpr char kDefaultCommandIndexPath[] =
    "/usr/share/sentia/command-index/command-index.json";
constexpr char kCommandIndexEnvOverride[] = "SENTIA_COMMAND_INDEX_PATH";

const std::regex kPackageNamePattern(kPackageNameRegex);
const std::regex kSha256Pattern("^[a-f0-9]{64}$");

struct WorkerError final : public std::runtime_error {
  explicit WorkerError(std::string code_in, std::string message_in,
                       json details_in = json::object())
      : std::runtime_error(std::move(message_in)),
        code(std::move(code_in)),
        details(std::move(details_in)) {}

  std::string code;
  json details;
};

struct RequestEnvelope {
  std::string request_id;
  std::string protocol_version;
  std::string operation;
  json arguments;
  std::optional<json> approval;
};

struct ApprovalEnvelope {
  std::string plan_digest;
  std::string broker_session_id;
  std::string authorization_id;
  std::string expires_at;
  bool allow_source_change = false;
  bool allow_essential_removal = false;
  bool allow_held_change = false;
};

struct PlannedTransaction {
  json canonical_plan;
  std::string digest;
  bool has_source_changes = false;
  bool has_essential_removals = false;
  bool has_held_changes = false;
};

std::string NullToEmpty(const char* value) {
  return value == nullptr ? std::string() : std::string(value);
}

std::string ToLower(std::string value) {
  std::transform(value.begin(), value.end(), value.begin(),
                 [](unsigned char c) { return static_cast<char>(std::tolower(c)); });
  return value;
}

bool CaseInsensitiveContains(const std::string& haystack,
                             const std::string& needle) {
  return ToLower(haystack).find(ToLower(needle)) != std::string::npos;
}

std::string FormatUtcNowIso8601() {
  const auto now = std::chrono::system_clock::now();
  const std::time_t now_time = std::chrono::system_clock::to_time_t(now);
  std::tm utc{};
  gmtime_r(&now_time, &utc);
  std::ostringstream out;
  out << std::put_time(&utc, "%Y-%m-%dT%H:%M:%SZ");
  return out.str();
}

std::optional<std::time_t> ParseIso8601Utc(const std::string& value) {
  std::tm parsed{};
  std::istringstream in(value);
  in >> std::get_time(&parsed, "%Y-%m-%dT%H:%M:%SZ");
  if (in.fail()) {
    return std::nullopt;
  }
  parsed.tm_isdst = 0;
  return timegm(&parsed);
}

std::string RequireStringField(const json& object, const std::string& field_name,
                               size_t max_length) {
  if (!object.contains(field_name)) {
    throw WorkerError("schema_validation_failed",
                      "Missing required string field",
                      {{"field", field_name}});
  }
  const json& value = object.at(field_name);
  if (!value.is_string()) {
    throw WorkerError("schema_validation_failed",
                      "Field must be a string",
                      {{"field", field_name}});
  }
  const std::string text = value.get<std::string>();
  if (text.empty() || text.size() > max_length) {
    throw WorkerError("schema_validation_failed",
                      "String field length is out of bounds",
                      {{"field", field_name},
                       {"min_length", 1},
                       {"max_length", max_length}});
  }
  return text;
}

std::string OptionalStringField(const json& object, const std::string& field_name,
                                const std::string& default_value,
                                size_t max_length) {
  if (!object.contains(field_name)) {
    return default_value;
  }
  const json& value = object.at(field_name);
  if (!value.is_string()) {
    throw WorkerError("schema_validation_failed",
                      "Optional field must be a string",
                      {{"field", field_name}});
  }
  const std::string text = value.get<std::string>();
  if (text.empty() || text.size() > max_length) {
    throw WorkerError("schema_validation_failed",
                      "Optional string field length is out of bounds",
                      {{"field", field_name},
                       {"min_length", 1},
                       {"max_length", max_length}});
  }
  return text;
}

bool OptionalBoolField(const json& object, const std::string& field_name,
                       bool default_value) {
  if (!object.contains(field_name)) {
    return default_value;
  }
  if (!object.at(field_name).is_boolean()) {
    throw WorkerError("schema_validation_failed",
                      "Optional field must be a boolean",
                      {{"field", field_name}});
  }
  return object.at(field_name).get<bool>();
}

int OptionalIntegerField(const json& object, const std::string& field_name,
                         int default_value, int min_value, int max_value) {
  if (!object.contains(field_name)) {
    return default_value;
  }
  if (!object.at(field_name).is_number_integer()) {
    throw WorkerError("schema_validation_failed",
                      "Optional field must be an integer",
                      {{"field", field_name}});
  }
  const int value = object.at(field_name).get<int>();
  if (value < min_value || value > max_value) {
    throw WorkerError("schema_validation_failed",
                      "Integer field is out of bounds",
                      {{"field", field_name},
                       {"min_value", min_value},
                       {"max_value", max_value}});
  }
  return value;
}

std::string NormalizeOperation(std::string op) {
  op = ToLower(std::move(op));
  if (op == "search") return "apt_search";
  if (op == "package_info") return "apt_package_info";
  if (op == "policy" || op == "package_policy") return "apt_package_policy";
  if (op == "simulate_install") return "apt_simulate_install";
  if (op == "install") return "apt_install";
  if (op == "remove") return "apt_remove";
  if (op == "update") return "apt_update";
  if (op == "upgrade") return "apt_upgrade";
  if (op == "packagefileownership") return "package_owns_file";
  return op;
}

void EnsurePackageName(const std::string& package_name) {
  if (!std::regex_match(package_name, kPackageNamePattern)) {
    throw WorkerError("schema_validation_failed", "Invalid package name syntax",
                      {{"package", package_name},
                       {"expected_regex", kPackageNameRegex}});
  }
}

std::vector<std::string> ParsePackagesArgument(const json& arguments,
                                               bool required) {
  if (!arguments.contains("packages")) {
    if (required) {
      throw WorkerError("schema_validation_failed",
                        "packages is required for this operation",
                        {{"field", "packages"}});
    }
    return {};
  }
  const json& packages = arguments.at("packages");
  if (!packages.is_array()) {
    throw WorkerError("schema_validation_failed", "packages must be an array",
                      {{"field", "packages"}});
  }
  if (packages.empty() || packages.size() > kMaxPackageCount) {
    throw WorkerError("schema_validation_failed",
                      "packages array length is out of bounds",
                      {{"field", "packages"},
                       {"min_items", 1},
                       {"max_items", kMaxPackageCount}});
  }

  std::vector<std::string> parsed;
  parsed.reserve(packages.size());
  std::unordered_set<std::string> seen;

  for (const auto& item : packages) {
    if (!item.is_string()) {
      throw WorkerError("schema_validation_failed",
                        "packages must contain only strings");
    }
    std::string package_name = item.get<std::string>();
    EnsurePackageName(package_name);
    if (seen.insert(package_name).second) {
      parsed.push_back(std::move(package_name));
    }
  }
  return parsed;
}

std::vector<std::string> DrainAptMessages() {
  std::vector<std::string> messages;
  size_t safety_counter = 0;
  while (!_error->empty(GlobalError::DEBUG)) {
    std::string text;
    _error->PopMessage(text);
    if (!text.empty()) {
      messages.push_back(std::move(text));
    }
    ++safety_counter;
    if (safety_counter > 1024) {
      break;
    }
  }
  return messages;
}

void ThrowIfAptError(const std::string& code, const std::string& context) {
  if (_error->PendingError()) {
    throw WorkerError(code, context, {{"apt_messages", DrainAptMessages()}});
  }
}

pkgCache::PkgIterator FindPackageByName(pkgCache& cache,
                                        const std::string& package_name) {
  pkgCache::PkgIterator package = cache.FindPkg(package_name);
  if (!package.end()) {
    return package;
  }
  pkgCache::GrpIterator group = cache.FindGrp(package_name);
  if (!group.end()) {
    package = group.FindPreferredPkg(true);
    if (!package.end()) {
      return package;
    }
  }
  return cache.FindPkg(package_name);
}

bool IsEssentialPackage(const pkgCache::PkgIterator& package) {
  return (package->Flags & pkgCache::Flag::Essential) != 0;
}

bool IsImportantPackage(const pkgCache::PkgIterator& package) {
  return (package->Flags & pkgCache::Flag::Important) != 0;
}

bool IsHeldPackage(const pkgCache::PkgIterator& package) {
  return package->SelectedState == pkgCache::State::Hold;
}

std::set<std::string> VersionOriginFingerprints(
    const pkgCache::VerIterator& version) {
  std::set<std::string> fingerprints;
  for (pkgCache::VerFileIterator file = version.FileList(); !file.end();
       ++file) {
    const pkgCache::PkgFileIterator pkg_file = file.File();
    std::ostringstream fingerprint;
    fingerprint << NullToEmpty(pkg_file.Origin()) << '|'
                << NullToEmpty(pkg_file.Archive()) << '|'
                << NullToEmpty(pkg_file.Codename()) << '|'
                << NullToEmpty(pkg_file.Site()) << '|'
                << NullToEmpty(pkg_file.Component());
    fingerprints.insert(fingerprint.str());
  }
  return fingerprints;
}

json CollectVersionOrigins(const pkgCache::VerIterator& version) {
  json origins = json::array();
  for (pkgCache::VerFileIterator file = version.FileList(); !file.end();
       ++file) {
    const pkgCache::PkgFileIterator pkg_file = file.File();
    origins.push_back(
        {{"origin", NullToEmpty(pkg_file.Origin())},
         {"archive", NullToEmpty(pkg_file.Archive())},
         {"codename", NullToEmpty(pkg_file.Codename())},
         {"label", NullToEmpty(pkg_file.Label())},
         {"site", NullToEmpty(pkg_file.Site())},
         {"component", NullToEmpty(pkg_file.Component())},
         {"architecture", NullToEmpty(pkg_file.Architecture())},
         {"index_type", NullToEmpty(pkg_file.IndexType())},
         {"index_file", NullToEmpty(pkg_file.FileName())}});
  }
  return origins;
}

json CollectDependencies(const pkgCache::VerIterator& version, int max_count) {
  json dependencies = json::array();
  int emitted = 0;
  for (pkgCache::DepIterator dependency = version.DependsList();
       !dependency.end(); ++dependency) {
    if (emitted >= max_count) {
      break;
    }
    const pkgCache::PkgIterator target = dependency.TargetPkg();
    dependencies.push_back(
        {{"target_package", NullToEmpty(target.Name())},
         {"dependency_type", NullToEmpty(dependency.DepType())},
         {"compare_operator", NullToEmpty(dependency.CompType())},
         {"target_version", NullToEmpty(dependency.TargetVer())},
         {"or_with_next",
          (dependency->CompareOp & pkgCache::Dep::Or) == pkgCache::Dep::Or}});
    ++emitted;
  }
  return dependencies;
}

std::string CanonicalizePlanDigest(const json& canonical_plan) {
  return Sha256Hex(canonical_plan.dump());
}

void SortChangeArray(json& changes) {
  std::sort(changes.begin(), changes.end(),
            [](const json& left, const json& right) {
              return left.at("package").get<std::string>() <
                     right.at("package").get<std::string>();
            });
}

class AcquireStatus final : public pkgAcquireStatus {
 public:
  explicit AcquireStatus(bool allow_release_info_changes)
      : allow_release_info_changes_(allow_release_info_changes) {}

  bool MediaChange(std::string, std::string) override { return false; }

  bool ReleaseInfoChanges(metaIndex const*, metaIndex const*,
                          std::vector<ReleaseInfoChange>&& changes) override {
    for (const auto& change : changes) {
      release_info_changes_.push_back(
          {{"type", change.Type},
           {"from", change.From},
           {"to", change.To},
           {"message", change.Message},
           {"default_action", change.DefaultAction}});
    }
    if (!allow_release_info_changes_) {
      return false;
    }
    return true;
  }

  const json& release_info_changes() const { return release_info_changes_; }

 private:
  bool allow_release_info_changes_;
  json release_info_changes_ = json::array();
};

class UnlockGuard final {
 public:
  explicit UnlockGuard(bool enabled) : enabled_(enabled) {}
  ~UnlockGuard() {
    if (enabled_ && _system != nullptr) {
      _system->LockInner();
    }
  }
  UnlockGuard(const UnlockGuard&) = delete;
  UnlockGuard& operator=(const UnlockGuard&) = delete;

 private:
  bool enabled_;
};

RequestEnvelope ParseRequest(const json& request) {
  if (!request.is_object()) {
    throw WorkerError("schema_validation_failed", "Request must be a JSON object");
  }

  RequestEnvelope envelope;
  envelope.request_id = RequireStringField(request, "request_id", kMaxRequestIdLength);
  envelope.protocol_version = RequireStringField(request, "protocol_version", 16);
  if (envelope.protocol_version != kProtocolVersion) {
    throw WorkerError("unsupported_protocol_version",
                      "protocol_version is not supported",
                      {{"expected", kProtocolVersion},
                       {"provided", envelope.protocol_version}});
  }

  envelope.operation = NormalizeOperation(RequireStringField(request, "operation", 64));
  if (!request.contains("arguments")) {
    envelope.arguments = json::object();
  } else if (!request.at("arguments").is_object()) {
    throw WorkerError("schema_validation_failed", "arguments must be an object",
                      {{"field", "arguments"}});
  } else {
    envelope.arguments = request.at("arguments");
  }

  if (request.contains("approval")) {
    if (request.at("approval").is_null()) {
      envelope.approval = std::nullopt;
    } else if (!request.at("approval").is_object()) {
      throw WorkerError("schema_validation_failed", "approval must be an object",
                        {{"field", "approval"}});
    } else {
      envelope.approval = request.at("approval");
    }
  }
  return envelope;
}

ApprovalEnvelope ParseApproval(const RequestEnvelope& request) {
  if (!request.approval.has_value()) {
    throw WorkerError("approval_required",
                      "execute mode requires broker approval payload");
  }
  const json& approval = request.approval.value();

  ApprovalEnvelope parsed;
  parsed.plan_digest = RequireStringField(approval, "plan_digest", 64);
  if (!std::regex_match(parsed.plan_digest, kSha256Pattern)) {
    throw WorkerError("schema_validation_failed",
                      "approval.plan_digest must be a lowercase sha256 hex digest");
  }
  parsed.broker_session_id =
      RequireStringField(approval, "broker_session_id", 128);
  parsed.authorization_id =
      RequireStringField(approval, "authorization_id", 128);
  parsed.expires_at = RequireStringField(approval, "expires_at", 64);

  parsed.allow_source_change =
      OptionalBoolField(approval, "allow_source_change", false);
  parsed.allow_essential_removal =
      OptionalBoolField(approval, "allow_essential_removal", false);
  parsed.allow_held_change = OptionalBoolField(approval, "allow_held_change", false);

  const std::optional<std::time_t> expires = ParseIso8601Utc(parsed.expires_at);
  if (!expires.has_value()) {
    throw WorkerError("schema_validation_failed",
                      "approval.expires_at must be UTC ISO-8601 with Z suffix",
                      {{"field", "approval.expires_at"}});
  }
  const std::time_t now = std::time(nullptr);
  if (expires.value() <= now) {
    throw WorkerError("approval_expired", "Approval is expired",
                      {{"expires_at", parsed.expires_at}});
  }

  return parsed;
}

void AssertRootForExecution(const RequestEnvelope& request) {
  if (geteuid() != 0) {
    throw WorkerError(
        "permission_denied",
        "execute mode requires a root-owned broker caller after authorization",
        {{"operation", request.operation}, {"effective_uid", geteuid()}});
  }
}

void ConfigureExecutionConffilePolicy() {
  _config->Clear("Dpkg::Options");
  _config->Set("Dpkg::Options::", "--force-confdef");
  _config->Set("Dpkg::Options::", "--force-confold");
  _config->Set("APT::Get::Assume-Yes", "true");
  _config->Set("APT::Color", "0");
  setenv("DEBIAN_FRONTEND", "noninteractive", 1);
}

json MakeVersionSummary(pkgDepCache* dep_cache, pkgCache::PkgIterator package,
                        pkgRecords* records = nullptr, int version_limit = 0) {
  json result = {{"name", NullToEmpty(package.Name())},
                 {"full_name", package.FullName(true)},
                 {"architecture", NullToEmpty(package.Arch())},
                 {"essential", IsEssentialPackage(package)},
                 {"important", IsImportantPackage(package)},
                 {"held", IsHeldPackage(package)}};

  const pkgCache::VerIterator installed = package.CurrentVer();
  if (!installed.end()) {
    result["installed_version"] = NullToEmpty(installed.VerStr());
  } else {
    result["installed_version"] = nullptr;
  }

  const pkgCache::VerIterator candidate = dep_cache->GetCandidateVersion(package);
  if (!candidate.end()) {
    result["candidate_version"] = NullToEmpty(candidate.VerStr());
  } else {
    result["candidate_version"] = nullptr;
  }

  if (!candidate.end()) {
    result["candidate_dependencies"] =
        CollectDependencies(candidate, kMaxDependencyEntries);
    result["candidate_origins"] = CollectVersionOrigins(candidate);

    if (records != nullptr) {
      pkgCache::VerFileIterator file = candidate.FileList();
      if (!file.end()) {
        pkgRecords::Parser& parser = records->Lookup(file);
        result["summary"] = parser.ShortDesc();
        result["homepage"] = parser.Homepage();
        result["maintainer"] = parser.Maintainer();
      }
    }
  } else {
    result["candidate_dependencies"] = json::array();
    result["candidate_origins"] = json::array();
  }

  if (version_limit > 0) {
    json versions = json::array();
    int emitted = 0;
    for (pkgCache::VerIterator version = package.VersionList(); !version.end();
         ++version) {
      if (emitted >= version_limit) {
        break;
      }
      versions.push_back({{"version", NullToEmpty(version.VerStr())},
                          {"architecture", NullToEmpty(version.Arch())},
                          {"priority", NullToEmpty(version.PriorityType())},
                          {"source_package", NullToEmpty(version.SourcePkgName())},
                          {"source_version", NullToEmpty(version.SourceVerStr())},
                          {"origins", CollectVersionOrigins(version)}});
      ++emitted;
    }
    result["versions"] = std::move(versions);
  }

  return result;
}

void MarkInstallPackages(pkgCache& cache, pkgDepCache* dep_cache,
                         const std::vector<std::string>& packages) {
  pkgDepCache::ActionGroup group(*dep_cache);
  for (const std::string& package_name : packages) {
    pkgCache::PkgIterator package = FindPackageByName(cache, package_name);
    if (package.end()) {
      throw WorkerError("package_not_found", "Requested package does not exist",
                        {{"package", package_name}});
    }
    if (!dep_cache->MarkInstall(package, true, 0, true)) {
      throw WorkerError(
          "transaction_resolution_failed", "MarkInstall failed",
          {{"package", package_name}, {"apt_messages", DrainAptMessages()}});
    }
  }
}

void MarkRemovePackages(pkgCache& cache, pkgDepCache* dep_cache,
                        const std::vector<std::string>& packages) {
  pkgDepCache::ActionGroup group(*dep_cache);
  for (const std::string& package_name : packages) {
    pkgCache::PkgIterator package = FindPackageByName(cache, package_name);
    if (package.end()) {
      throw WorkerError("package_not_found", "Requested package does not exist",
                        {{"package", package_name}});
    }
    if (!dep_cache->MarkDelete(package, false, 0, true)) {
      throw WorkerError("transaction_resolution_failed", "MarkDelete failed",
                        {{"package", package_name},
                         {"apt_messages", DrainAptMessages()}});
    }
  }
}

void ResolveTransaction(pkgDepCache* dep_cache) {
  if (dep_cache->BrokenCount() == 0 && dep_cache->PolicyBrokenCount() == 0) {
    return;
  }

  pkgProblemResolver resolver(dep_cache);
  if (!resolver.Resolve(true)) {
    throw WorkerError("transaction_resolution_failed",
                      "Dependency resolver reported failure",
                      {{"apt_messages", DrainAptMessages()}});
  }
  if (dep_cache->BrokenCount() != 0 || dep_cache->PolicyBrokenCount() != 0) {
    throw WorkerError("transaction_resolution_failed",
                      "Transaction remains in a broken state after resolve",
                      {{"broken_count", dep_cache->BrokenCount()},
                       {"policy_broken_count", dep_cache->PolicyBrokenCount()}});
  }
}

PlannedTransaction BuildCanonicalPlan(const std::string& operation,
                                      const std::vector<std::string>& requested,
                                      pkgCache& cache, pkgDepCache* dep_cache,
                                      bool with_lock) {
  json install = json::array();
  json remove = json::array();
  json upgrade = json::array();
  json downgrade = json::array();
  json held_changes = json::array();
  json source_changes = json::array();
  json dependency_changes = json::array();
  json essential_removals = json::array();

  bool source_detection_complete = true;

  for (pkgCache::PkgIterator package = cache.PkgBegin(); !package.end();
       ++package) {
    pkgDepCache::StateCache& state = (*dep_cache)[package];
    if (!(state.Install() || state.Delete())) {
      continue;
    }

    const pkgCache::VerIterator installed = package.CurrentVer();
    const pkgCache::VerIterator planned = state.InstVerIter(cache);
    const pkgCache::VerIterator candidate = state.CandidateVerIter(cache);

    json change = {
        {"package", NullToEmpty(package.Name())},
        {"full_name", package.FullName(true)},
        {"architecture", NullToEmpty(package.Arch())},
        {"from_version", installed.end() ? json(nullptr) : json(installed.VerStr())},
        {"to_version", planned.end() ? json(nullptr) : json(planned.VerStr())},
        {"candidate_version",
         candidate.end() ? json(nullptr) : json(candidate.VerStr())},
        {"held", IsHeldPackage(package)},
        {"essential", IsEssentialPackage(package)},
        {"important", IsImportantPackage(package)}};

    if (state.Delete()) {
      remove.push_back(change);
      if (IsEssentialPackage(package) || IsImportantPackage(package)) {
        essential_removals.push_back(change);
      }
    } else if (state.Downgrade()) {
      downgrade.push_back(change);
    } else if (installed.end() || !state.Upgrade()) {
      install.push_back(change);
    } else {
      upgrade.push_back(change);
    }

    if (IsHeldPackage(package)) {
      held_changes.push_back(change);
    }

    if (!planned.end()) {
      dependency_changes.push_back(
          {{"package", NullToEmpty(package.Name())},
           {"to_version", NullToEmpty(planned.VerStr())},
           {"dependencies", CollectDependencies(planned, kMaxDependencyEntries)}});
    }

    if (!installed.end() && !planned.end()) {
      const std::set<std::string> installed_origins =
          VersionOriginFingerprints(installed);
      const std::set<std::string> planned_origins =
          VersionOriginFingerprints(planned);
      if (installed_origins.empty() || planned_origins.empty()) {
        source_detection_complete = false;
      } else if (installed_origins != planned_origins) {
        source_changes.push_back(
            {{"package", NullToEmpty(package.Name())},
             {"from_origins", CollectVersionOrigins(installed)},
             {"to_origins", CollectVersionOrigins(planned)}});
      }
    }
  }

  SortChangeArray(install);
  SortChangeArray(remove);
  SortChangeArray(upgrade);
  SortChangeArray(downgrade);
  SortChangeArray(held_changes);
  SortChangeArray(source_changes);
  SortChangeArray(essential_removals);
  SortChangeArray(dependency_changes);

  std::vector<std::string> sorted_requested = requested;
  std::sort(sorted_requested.begin(), sorted_requested.end());

  PlannedTransaction planned;
  planned.canonical_plan =
      json{{"protocol_version", kProtocolVersion},
           {"operation", operation},
           {"requested_packages", sorted_requested},
           {"with_lock", with_lock},
           {"changes",
            {{"install", install},
             {"remove", remove},
             {"upgrade", upgrade},
             {"downgrade", downgrade}}},
           {"dependency_changes", dependency_changes},
           {"held_changes", held_changes},
           {"essential_removals", essential_removals},
           {"source_changes", source_changes},
           {"source_change_detection_complete", source_detection_complete},
           {"download_bytes", dep_cache->DebSize()},
           {"disk_bytes_delta", dep_cache->UsrSize()},
           {"counts",
            {{"install", dep_cache->InstCount()},
             {"remove", dep_cache->DelCount()},
             {"keep", dep_cache->KeepCount()},
             {"broken", dep_cache->BrokenCount()},
             {"policy_broken", dep_cache->PolicyBrokenCount()}}},
           {"service_effects",
            {{"known", json::array()},
             {"unknown", true},
             {"note",
              "Service impacts from maintainer scripts are not determinable "
              "from package metadata alone."}}},
           {"conffile_policy",
            {{"dpkg_options", {"--force-confdef", "--force-confold"}},
             {"preserve_local_configs", true}}},
           {"debconf_policy",
            {{"frontend", "noninteractive"},
             {"note", "Broker must define debconf environment explicitly"}}}};

  planned.digest = CanonicalizePlanDigest(planned.canonical_plan);
  planned.has_source_changes = !source_changes.empty();
  planned.has_essential_removals = !essential_removals.empty();
  planned.has_held_changes = !held_changes.empty();
  return planned;
}

json ExecuteResolvedPlan(pkgCacheFile& cache_file, pkgDepCache* dep_cache) {
  ConfigureExecutionConffilePolicy();
  if (_system == nullptr) {
    throw WorkerError("transaction_prepare_failed",
                      "APT system backend is not initialized");
  }

  if (!cache_file.BuildSourceList()) {
    throw WorkerError("transaction_prepare_failed",
                      "Unable to build package source list",
                      {{"apt_messages", DrainAptMessages()}});
  }
  pkgSourceList* source_list = cache_file.GetSourceList();
  if (source_list == nullptr) {
    throw WorkerError("transaction_prepare_failed",
                      "Source list is not available");
  }

  pkgRecords records(*cache_file.GetPkgCache());
  ThrowIfAptError("transaction_prepare_failed",
                  "Unable to prepare package record parser");

  pkgAcquire fetcher;
  if (!fetcher.GetLock(_config->FindDir("Dir::Cache::Archives"))) {
    throw WorkerError("transaction_prepare_failed",
                      "Unable to lock package archive cache");
  }

  std::unique_ptr<pkgPackageManager> manager(_system->CreatePM(dep_cache));
  if (!manager) {
    throw WorkerError("transaction_prepare_failed",
                      "Unable to create libapt package manager");
  }

  if (!manager->GetArchives(&fetcher, source_list, &records)) {
    throw WorkerError("transaction_prepare_failed",
                      "Unable to resolve transaction archive set",
                      {{"apt_messages", DrainAptMessages()}});
  }

  const pkgAcquire::RunResult fetch_result = fetcher.Run();
  if (fetch_result == pkgAcquire::Cancelled) {
    throw WorkerError("transaction_cancelled", "Archive acquisition cancelled");
  }
  if (fetch_result == pkgAcquire::Failed) {
    throw WorkerError("transaction_fetch_failed",
                      "Archive acquisition failed",
                      {{"apt_messages", DrainAptMessages()}});
  }

  bool unlocked_inner = false;
  if (_system != nullptr) {
    if (!_system->UnLockInner()) {
      throw WorkerError("transaction_prepare_failed",
                        "Unable to release inner dpkg lock");
    }
    unlocked_inner = true;
  }
  UnlockGuard relock(unlocked_inner);

  APT::Progress::PackageManager progress;
  const pkgPackageManager::OrderResult order_result = manager->DoInstall(&progress);

  json execution = {
      {"fetch_result", "continue"},
      {"order_result",
       order_result == pkgPackageManager::Completed
           ? "completed"
           : (order_result == pkgPackageManager::Incomplete ? "incomplete"
                                                            : "failed")},
      {"interrupted", order_result == pkgPackageManager::Incomplete ||
                          pkgPackageManager::SigINTStop},
      {"disappeared_packages", json::array()}};

  for (const std::string& package : manager->GetDisappearedPackages()) {
    execution["disappeared_packages"].push_back(package);
  }

  if (order_result == pkgPackageManager::Completed && !_error->PendingError()) {
    return execution;
  }

  execution["apt_messages"] = DrainAptMessages();
  if (order_result == pkgPackageManager::Incomplete ||
      pkgPackageManager::SigINTStop) {
    throw WorkerError("transaction_interrupted",
                      "Package transaction was interrupted",
                      execution);
  }
  throw WorkerError("transaction_failed", "Package transaction failed",
                    execution);
}

json BuildSourcesManifest() {
  json files = json::array();

  std::vector<std::filesystem::path> source_files;
  const std::filesystem::path main_source("/etc/apt/sources.list");
  if (std::filesystem::exists(main_source)) {
    source_files.push_back(main_source);
  }
  const std::filesystem::path source_dir("/etc/apt/sources.list.d");
  if (std::filesystem::exists(source_dir)) {
    for (const auto& entry : std::filesystem::directory_iterator(source_dir)) {
      if (!entry.is_regular_file()) {
        continue;
      }
      const std::string extension = entry.path().extension().string();
      if (extension == ".list" || extension == ".sources") {
        source_files.push_back(entry.path());
      }
    }
  }

  std::sort(source_files.begin(), source_files.end());
  for (const auto& path : source_files) {
    std::ifstream in(path, std::ios::binary);
    if (!in) {
      throw WorkerError("apt_sources_unreadable", "Unable to read apt source file",
                        {{"path", path.string()}});
    }
    std::ostringstream contents;
    contents << in.rdbuf();
    const std::string text = contents.str();
    files.push_back({{"path", path.string()},
                     {"bytes", text.size()},
                     {"sha256", Sha256Hex(text)}});
  }

  return files;
}

std::string ResolveCommandIndexPath(const json& arguments) {
  if (arguments.contains("index_path")) {
    return RequireStringField(arguments, "index_path", kMaxFilePathLength);
  }
  const char* env_path = std::getenv(kCommandIndexEnvOverride);
  if (env_path != nullptr && std::string(env_path).size() <= kMaxFilePathLength) {
    return std::string(env_path);
  }
  return kDefaultCommandIndexPath;
}

json LoadCommandIndex(const std::string& path) {
  const std::filesystem::path index_path(path);
  if (!std::filesystem::exists(index_path)) {
    throw WorkerError("command_index_missing", "Command index file not found",
                      {{"path", path}});
  }
  const uintmax_t bytes = std::filesystem::file_size(index_path);
  if (bytes > kMaxCommandIndexSizeBytes) {
    throw WorkerError("command_index_too_large", "Command index exceeds limit",
                      {{"path", path},
                       {"bytes", bytes},
                       {"max_bytes", kMaxCommandIndexSizeBytes}});
  }

  std::ifstream in(index_path);
  if (!in) {
    throw WorkerError("command_index_unreadable",
                      "Unable to open command index file", {{"path", path}});
  }
  json parsed;
  try {
    in >> parsed;
  } catch (const std::exception& error) {
    throw WorkerError("command_index_invalid_json",
                      "Command index file is not valid JSON",
                      {{"path", path}, {"what", error.what()}});
  }

  if (!parsed.is_object() || !parsed.contains("commands") ||
      !parsed.at("commands").is_object()) {
    throw WorkerError("command_index_invalid_schema",
                      "Command index JSON must contain object 'commands'",
                      {{"path", path}});
  }
  return parsed;
}

json HandleSearch(const RequestEnvelope& request) {
  const std::string query =
      RequireStringField(request.arguments, "query", kMaxSearchQueryLength);
  const int limit = OptionalIntegerField(request.arguments, "limit",
                                         kDefaultSearchLimit, 1, kMaxSearchLimit);

  pkgCacheFile cache_file;
  if (!cache_file.Open(nullptr, false)) {
    throw WorkerError("apt_cache_open_failed",
                      "Unable to open apt cache for search",
                      {{"apt_messages", DrainAptMessages()}});
  }
  pkgCache* cache = cache_file.GetPkgCache();
  pkgDepCache* dep_cache = cache_file.GetDepCache();
  if (cache == nullptr || dep_cache == nullptr) {
    throw WorkerError("apt_cache_open_failed",
                      "Apt cache/depcache pointer is null");
  }

  pkgRecords records(*cache);
  ThrowIfAptError("apt_cache_open_failed", "Unable to read package records");

  json matches = json::array();
  for (pkgCache::PkgIterator package = cache->PkgBegin();
       !package.end() && static_cast<int>(matches.size()) < limit; ++package) {
    const std::string name = NullToEmpty(package.Name());
    std::string summary;
    pkgCache::VerIterator candidate = dep_cache->GetCandidateVersion(package);
    if (!candidate.end()) {
      pkgCache::VerFileIterator file = candidate.FileList();
      if (!file.end()) {
        pkgRecords::Parser& parser = records.Lookup(file);
        summary = parser.ShortDesc();
      }
    }

    if (!CaseInsensitiveContains(name, query) &&
        !CaseInsensitiveContains(summary, query)) {
      continue;
    }

    matches.push_back({{"name", name},
                       {"full_name", package.FullName(true)},
                       {"architecture", NullToEmpty(package.Arch())},
                       {"summary", summary},
                       {"installed_version",
                        package.CurrentVer().end()
                            ? json(nullptr)
                            : json(package.CurrentVer().VerStr())},
                       {"candidate_version",
                        candidate.end() ? json(nullptr) : json(candidate.VerStr())}});
  }

  return {{"query", query}, {"limit", limit}, {"matches", matches}};
}

json HandlePackageInfo(const RequestEnvelope& request) {
  const std::vector<std::string> packages =
      ParsePackagesArgument(request.arguments, true);
  const int version_limit = OptionalIntegerField(
      request.arguments, "version_limit", kDefaultVersionLimit, 1,
      kMaxVersionLimit);

  pkgCacheFile cache_file;
  if (!cache_file.Open(nullptr, false)) {
    throw WorkerError("apt_cache_open_failed",
                      "Unable to open apt cache for package info",
                      {{"apt_messages", DrainAptMessages()}});
  }
  pkgCache* cache = cache_file.GetPkgCache();
  pkgDepCache* dep_cache = cache_file.GetDepCache();
  if (cache == nullptr || dep_cache == nullptr) {
    throw WorkerError("apt_cache_open_failed",
                      "Apt cache/depcache pointer is null");
  }

  pkgRecords records(*cache);
  ThrowIfAptError("apt_cache_open_failed", "Unable to read package records");

  json info = json::array();
  for (const std::string& package_name : packages) {
    pkgCache::PkgIterator package = FindPackageByName(*cache, package_name);
    if (package.end()) {
      throw WorkerError("package_not_found", "Requested package does not exist",
                        {{"package", package_name}});
    }
    info.push_back(
        MakeVersionSummary(dep_cache, package, &records, version_limit));
  }

  return {{"packages", info}};
}

json HandlePackagePolicy(const RequestEnvelope& request) {
  const std::vector<std::string> packages =
      ParsePackagesArgument(request.arguments, true);
  const int version_limit = OptionalIntegerField(
      request.arguments, "version_limit", kDefaultVersionLimit, 1,
      kMaxVersionLimit);

  pkgCacheFile cache_file;
  if (!cache_file.Open(nullptr, false)) {
    throw WorkerError("apt_cache_open_failed",
                      "Unable to open apt cache for policy",
                      {{"apt_messages", DrainAptMessages()}});
  }
  pkgCache* cache = cache_file.GetPkgCache();
  pkgDepCache* dep_cache = cache_file.GetDepCache();
  pkgPolicy* policy = cache_file.GetPolicy();
  if (cache == nullptr || dep_cache == nullptr || policy == nullptr) {
    throw WorkerError("apt_cache_open_failed",
                      "Apt cache/policy pointer is null");
  }

  json results = json::array();
  for (const std::string& package_name : packages) {
    pkgCache::PkgIterator package = FindPackageByName(*cache, package_name);
    if (package.end()) {
      throw WorkerError("package_not_found", "Requested package does not exist",
                        {{"package", package_name}});
    }

    const pkgCache::VerIterator candidate = dep_cache->GetCandidateVersion(package);
    json versions = json::array();
    int emitted = 0;
    for (pkgCache::VerIterator version = package.VersionList(); !version.end();
         ++version) {
      if (emitted >= version_limit) {
        break;
      }
      versions.push_back({{"version", NullToEmpty(version.VerStr())},
                          {"priority", policy->GetPriority(version)},
                          {"is_candidate",
                           !candidate.end() &&
                               NullToEmpty(version.VerStr()) ==
                                   NullToEmpty(candidate.VerStr())},
                          {"origins", CollectVersionOrigins(version)}});
      ++emitted;
    }

    results.push_back(
        {{"name", NullToEmpty(package.Name())},
         {"full_name", package.FullName(true)},
         {"installed_version",
          package.CurrentVer().end() ? json(nullptr)
                                     : json(package.CurrentVer().VerStr())},
         {"candidate_version",
          candidate.end() ? json(nullptr) : json(candidate.VerStr())},
         {"versions", versions}});
  }

  return {{"packages", results}};
}

PlannedTransaction BuildPlannedTransaction(const RequestEnvelope& request,
                                           bool with_lock,
                                           bool require_packages,
                                           const std::string& operation) {
  const std::vector<std::string> packages =
      ParsePackagesArgument(request.arguments, require_packages);

  pkgCacheFile cache_file;
  if (!cache_file.Open(nullptr, with_lock)) {
    throw WorkerError("apt_cache_open_failed",
                      "Unable to open apt cache for transaction planning",
                      {{"with_lock", with_lock},
                       {"apt_messages", DrainAptMessages()}});
  }
  pkgCache* cache = cache_file.GetPkgCache();
  pkgDepCache* dep_cache = cache_file.GetDepCache();
  if (cache == nullptr || dep_cache == nullptr) {
    throw WorkerError("apt_cache_open_failed",
                      "Apt cache/depcache pointer is null");
  }

  if (operation == "apt_install" || operation == "apt_simulate_install") {
    MarkInstallPackages(*cache, dep_cache, packages);
  } else if (operation == "apt_remove") {
    MarkRemovePackages(*cache, dep_cache, packages);
  } else if (operation == "apt_upgrade") {
    if (!APT::Upgrade::Upgrade(*dep_cache, APT::Upgrade::ALLOW_EVERYTHING)) {
      throw WorkerError("transaction_resolution_failed",
                        "APT::Upgrade failed",
                        {{"apt_messages", DrainAptMessages()}});
    }
  }

  ResolveTransaction(dep_cache);
  return BuildCanonicalPlan(operation, packages, *cache, dep_cache, with_lock);
}

json HandlePlanOnly(const RequestEnvelope& request, const std::string& operation,
                    bool require_packages) {
  PlannedTransaction planned =
      BuildPlannedTransaction(request, false, require_packages, operation);
  return {{"mode", "plan"},
          {"canonical_plan", planned.canonical_plan},
          {"plan_digest", planned.digest},
          {"execution_guards",
           {{"requires_root", true},
            {"requires_approval", true},
            {"reject_unapproved_source_change", true},
            {"reject_essential_removal", true},
            {"reject_held_change", true}}}};
}

json HandleExecute(const RequestEnvelope& request, const std::string& operation,
                   bool require_packages) {
  AssertRootForExecution(request);
  const ApprovalEnvelope approval = ParseApproval(request);

  PlannedTransaction planned =
      BuildPlannedTransaction(request, true, require_packages, operation);

  if (planned.digest != approval.plan_digest) {
    throw WorkerError("plan_digest_mismatch",
                      "Canonical plan digest changed; renewed approval required",
                      {{"expected_digest", approval.plan_digest},
                       {"actual_digest", planned.digest},
                       {"canonical_plan", planned.canonical_plan}});
  }

  if (planned.has_source_changes && !approval.allow_source_change) {
    throw WorkerError("source_change_requires_approval",
                      "Plan includes source changes without explicit approval",
                      {{"plan_digest", planned.digest},
                       {"source_changes", planned.canonical_plan.at("source_changes")}});
  }
  if (planned.has_essential_removals && !approval.allow_essential_removal) {
    throw WorkerError(
        "essential_removal_rejected",
        "Plan removes essential/important packages without explicit approval",
        {{"plan_digest", planned.digest},
         {"essential_removals",
          planned.canonical_plan.at("essential_removals")}});
  }
  if (planned.has_held_changes && !approval.allow_held_change) {
    throw WorkerError("held_package_change_rejected",
                      "Plan changes held packages without explicit approval",
                      {{"plan_digest", planned.digest},
                       {"held_changes", planned.canonical_plan.at("held_changes")}});
  }

  pkgCacheFile cache_file;
  if (!cache_file.Open(nullptr, true)) {
    throw WorkerError("apt_cache_open_failed",
                      "Unable to lock apt cache for execution",
                      {{"apt_messages", DrainAptMessages()}});
  }
  pkgCache* cache = cache_file.GetPkgCache();
  pkgDepCache* dep_cache = cache_file.GetDepCache();
  if (cache == nullptr || dep_cache == nullptr) {
    throw WorkerError("apt_cache_open_failed",
                      "Apt cache/depcache pointer is null");
  }

  const std::vector<std::string> packages =
      ParsePackagesArgument(request.arguments, require_packages);
  if (operation == "apt_install") {
    MarkInstallPackages(*cache, dep_cache, packages);
  } else if (operation == "apt_remove") {
    MarkRemovePackages(*cache, dep_cache, packages);
  } else if (operation == "apt_upgrade") {
    if (!APT::Upgrade::Upgrade(*dep_cache, APT::Upgrade::ALLOW_EVERYTHING)) {
      throw WorkerError("transaction_resolution_failed",
                        "APT::Upgrade failed in execute stage",
                        {{"apt_messages", DrainAptMessages()}});
    }
  }
  ResolveTransaction(dep_cache);

  PlannedTransaction execute_plan =
      BuildCanonicalPlan(operation, packages, *cache, dep_cache, true);
  if (execute_plan.digest != approval.plan_digest) {
    throw WorkerError(
        "plan_digest_mismatch",
        "Re-resolved execution plan differs from approved digest",
        {{"expected_digest", approval.plan_digest},
         {"actual_digest", execute_plan.digest},
         {"canonical_plan", execute_plan.canonical_plan}});
  }

  json execution = ExecuteResolvedPlan(cache_file, dep_cache);
  return {{"mode", "execute"},
          {"plan_digest", execute_plan.digest},
          {"canonical_plan", execute_plan.canonical_plan},
          {"approval",
           {{"broker_session_id", approval.broker_session_id},
            {"authorization_id", approval.authorization_id},
            {"expires_at", approval.expires_at},
            {"allow_source_change", approval.allow_source_change},
            {"allow_essential_removal", approval.allow_essential_removal},
            {"allow_held_change", approval.allow_held_change}}},
          {"execution", execution}};
}

json HandleMutatingPlanExecute(const RequestEnvelope& request,
                               const std::string& operation,
                               bool require_packages) {
  const std::string mode =
      OptionalStringField(request.arguments, "mode", "plan", 16);
  if (mode != "plan" && mode != "execute") {
    throw WorkerError("schema_validation_failed", "mode must be plan or execute",
                      {{"field", "arguments.mode"}});
  }
  return mode == "plan" ? HandlePlanOnly(request, operation, require_packages)
                        : HandleExecute(request, operation, require_packages);
}

json HandleUpdate(const RequestEnvelope& request) {
  const std::string mode =
      OptionalStringField(request.arguments, "mode", "plan", 16);
  if (mode != "plan" && mode != "execute") {
    throw WorkerError("schema_validation_failed", "mode must be plan or execute",
                      {{"field", "arguments.mode"}});
  }

  json canonical_plan = {{"protocol_version", kProtocolVersion},
                         {"operation", "apt_update"},
                         {"sources_manifest", BuildSourcesManifest()},
                         {"requires_release_info_change_approval", true}};
  const std::string plan_digest = CanonicalizePlanDigest(canonical_plan);

  if (mode == "plan") {
    return {{"mode", "plan"},
            {"plan_digest", plan_digest},
            {"canonical_plan", canonical_plan},
            {"execution_guards",
             {{"requires_root", true},
              {"requires_approval", true},
              {"reject_unapproved_source_change", true}}}};
  }

  AssertRootForExecution(request);
  const ApprovalEnvelope approval = ParseApproval(request);
  if (approval.plan_digest != plan_digest) {
    throw WorkerError("plan_digest_mismatch",
                      "APT source manifest changed before update execution",
                      {{"expected_digest", approval.plan_digest},
                       {"actual_digest", plan_digest},
                       {"canonical_plan", canonical_plan}});
  }

  if (_system != nullptr && !_system->Lock()) {
    throw WorkerError("apt_lock_failed", "Failed to lock package system for update");
  }
  struct SystemUnlockGuard final {
    ~SystemUnlockGuard() {
      if (_system != nullptr) {
        _system->UnLock(true);
      }
    }
  } unlock_guard;

  pkgSourceList source_list;
  if (!source_list.ReadMainList()) {
    throw WorkerError("apt_update_failed", "Unable to read apt source list",
                      {{"apt_messages", DrainAptMessages()}});
  }

  AcquireStatus status(approval.allow_source_change);
  if (!ListUpdate(status, source_list, 0)) {
    throw WorkerError("apt_update_failed", "ListUpdate failed",
                      {{"release_info_changes", status.release_info_changes()},
                       {"apt_messages", DrainAptMessages()}});
  }

  return {{"mode", "execute"},
          {"plan_digest", plan_digest},
          {"canonical_plan", canonical_plan},
          {"release_info_changes", status.release_info_changes()},
          {"execution", {{"updated", true}}}};
}

json HandlePackageOwnsFile(const RequestEnvelope& request) {
  const std::string file_path =
      RequireStringField(request.arguments, "file_path", kMaxFilePathLength);
  const std::string index_path = ResolveCommandIndexPath(request.arguments);
  const json command_index = LoadCommandIndex(index_path);

  const std::filesystem::path normalized(file_path);
  const std::string command = normalized.filename().string();
  if (command.empty()) {
    throw WorkerError("schema_validation_failed",
                      "file_path must include a final path component");
  }

  const json& commands = command_index.at("commands");
  if (!commands.contains(command)) {
    return {{"file_path", file_path},
            {"command", command},
            {"found", false},
            {"packages", json::array()},
            {"paths", json::array()},
            {"index_path", index_path},
            {"provenance",
             command_index.contains("provenance") ? command_index.at("provenance")
                                                  : json(nullptr)}};
  }

  const json& match = commands.at(command);
  if (!match.is_object()) {
    throw WorkerError("command_index_invalid_schema",
                      "command entry must be an object",
                      {{"command", command}, {"index_path", index_path}});
  }

  return {{"file_path", file_path},
          {"command", command},
          {"found", true},
          {"packages", match.contains("packages") ? match.at("packages")
                                                  : json::array()},
          {"paths", match.contains("paths") ? match.at("paths") : json::array()},
          {"index_path", index_path},
          {"provenance",
           command_index.contains("provenance") ? command_index.at("provenance")
                                                : json(nullptr)}};
}

json HandleDiagnostics() {
  json command_index = json::object();
  std::string command_index_path = kDefaultCommandIndexPath;
  const char* env_override = std::getenv(kCommandIndexEnvOverride);
  if (env_override != nullptr && std::string(env_override).size() <= kMaxFilePathLength) {
    command_index_path = std::string(env_override);
  }
  command_index["path"] = command_index_path;
  command_index["exists"] = std::filesystem::exists(command_index_path);

  return {
      {"effective_uid", geteuid()},
      {"supports_mutation", geteuid() == 0},
      {"apt_pkg_version", pkgVersion == nullptr ? "" : std::string(pkgVersion)},
      {"apt_lib_version",
       pkgLibVersion == nullptr ? "" : std::string(pkgLibVersion)},
      {"protocol_version", kProtocolVersion},
      {"supported_operations",
       {"apt_search", "apt_package_info", "apt_package_policy",
        "apt_simulate_install", "apt_install", "apt_remove", "apt_update",
        "apt_upgrade", "package_owns_file", "diagnostics"}},
      {"execution_path",
       {{"requires_approval", true},
        {"requires_root", true},
        {"transaction_executor", "libapt-pkg pkgPackageManager"},
        {"no_shell_execution", true}}},
      {"command_index", command_index},
  };
}

json Dispatch(const RequestEnvelope& request) {
  if (request.operation == "apt_search") {
    return HandleSearch(request);
  }
  if (request.operation == "apt_package_info") {
    return HandlePackageInfo(request);
  }
  if (request.operation == "apt_package_policy") {
    return HandlePackagePolicy(request);
  }
  if (request.operation == "apt_simulate_install") {
    return HandlePlanOnly(request, request.operation, true);
  }
  if (request.operation == "apt_install") {
    return HandleMutatingPlanExecute(request, request.operation, true);
  }
  if (request.operation == "apt_remove") {
    return HandleMutatingPlanExecute(request, request.operation, true);
  }
  if (request.operation == "apt_upgrade") {
    return HandleMutatingPlanExecute(request, request.operation, false);
  }
  if (request.operation == "apt_update") {
    return HandleUpdate(request);
  }
  if (request.operation == "package_owns_file") {
    return HandlePackageOwnsFile(request);
  }
  if (request.operation == "diagnostics") {
    return HandleDiagnostics();
  }

  throw WorkerError("unsupported_operation", "Operation is not allowlisted",
                    {{"operation", request.operation}});
}

json BuildBaseResponse(const RequestEnvelope& request) {
  return {{"request_id", request.request_id},
          {"protocol_version", kProtocolVersion},
          {"operation", request.operation},
          {"timestamp", FormatUtcNowIso8601()}};
}

}  // namespace

AptWorker::AptWorker() {
  if (!pkgInitConfig(*_config)) {
    throw std::runtime_error("Failed to initialize apt configuration");
  }
  if (!pkgInitSystem(*_config, _system)) {
    throw std::runtime_error("Failed to initialize apt system");
  }

  _config->Set("APT::Color", "0");
  _config->Set("Acquire::AllowInsecureRepositories", "false");
  _config->Set("Acquire::AllowDowngradeToInsecureRepositories", "false");
}

json AptWorker::Process(const json& request_json) {
  try {
    RequestEnvelope request = ParseRequest(request_json);
    json response = BuildBaseResponse(request);
    _error->Discard();

    json result = Dispatch(request);
    response["status"] = "ok";
    response["result"] = std::move(result);
    const std::vector<std::string> apt_messages = DrainAptMessages();
    if (!apt_messages.empty()) {
      response["diagnostics"]["apt_messages"] = apt_messages;
    }
    return response;
  } catch (const WorkerError& error) {
    const json request_id =
        request_json.is_object() && request_json.contains("request_id")
            ? request_json.at("request_id")
            : json(nullptr);
    const json operation =
        request_json.is_object() && request_json.contains("operation")
            ? request_json.at("operation")
            : json(nullptr);

    json response = {{"request_id", request_id},
                     {"protocol_version", kProtocolVersion},
                     {"operation", operation},
                     {"timestamp", FormatUtcNowIso8601()},
                     {"status", "error"},
                     {"error", {{"code", error.code}, {"message", error.what()}}}};
    if (!error.details.empty()) {
      response["error"]["details"] = error.details;
    }
    const std::vector<std::string> apt_messages = DrainAptMessages();
    if (!apt_messages.empty()) {
      response["diagnostics"]["apt_messages"] = apt_messages;
    }
    return response;
  } catch (const std::exception& error) {
    const json request_id =
        request_json.is_object() && request_json.contains("request_id")
            ? request_json.at("request_id")
            : json(nullptr);
    const json operation =
        request_json.is_object() && request_json.contains("operation")
            ? request_json.at("operation")
            : json(nullptr);

    json response = {{"request_id", request_id},
                     {"protocol_version", kProtocolVersion},
                     {"operation", operation},
                     {"timestamp", FormatUtcNowIso8601()},
                     {"status", "error"},
                     {"error",
                      {{"code", "internal_error"}, {"message", error.what()}}}};

    const std::vector<std::string> apt_messages = DrainAptMessages();
    if (!apt_messages.empty()) {
      response["diagnostics"]["apt_messages"] = apt_messages;
    }
    return response;
  }
}

}  // namespace sentia::apt
