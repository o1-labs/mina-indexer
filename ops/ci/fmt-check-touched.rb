#!/usr/bin/env ruby
# frozen_string_literal: true

# "Format only what you touch" gate.
#
# Fails only if a line CHANGED in this branch (vs the base ref) is not
# rustfmt-clean. Pre-existing drift in files (or lines) you did not touch is
# ignored on purpose — a plain `cargo fmt --check` would produce the large,
# unrelated diffs CLAUDE.md warns against. Uses the repo's nightly rustfmt and
# rust/rustfmt.toml.
#
# Usage: ops/ci/fmt-check-touched.rb [base-ref]   (default: origin/main)

TOP = File.expand_path("../..", __dir__)
Dir.chdir(TOP)

base = ARGV[0] || "origin/main"
edition = "2021"
config = "rust/rustfmt.toml"

# 1. changed .rs files -> the new-side line ranges they touch
changed = Hash.new { |h, k| h[k] = [] }
current = nil
IO.popen(["git", "diff", "--unified=0", "#{base}...HEAD", "--", "*.rs"]) do |io|
  io.each_line do |line|
    if line.start_with?("+++ b/")
      path = line[6..].strip
      current = (path == "/dev/null") ? nil : path
    elsif current && line.start_with?("@@") && line =~ /\+(\d+)(?:,(\d+))?/
      start = Regexp.last_match(1).to_i
      count = Regexp.last_match(2) ? Regexp.last_match(2).to_i : 1
      changed[current] << (start..(start + count - 1)) unless count.zero?
    end
  end
end

if changed.empty?
  puts "fmt-check: no changed Rust files."
  exit 0
end

# 2. for each changed file, ask rustfmt which lines it would reformat, and keep
#    only the ones that fall inside a line the branch actually changed.
violations = []
changed.each_key do |file|
  next unless File.exist?(file)

  out = `rustfmt --check --edition #{edition} --config-path #{config} #{file} 2>&1`
  out.each_line do |l|
    next unless l =~ /^Diff in .+:(\d+):/

    diff_line = Regexp.last_match(1).to_i
    violations << "#{file}:#{diff_line}" if changed[file].any? { |r| r.include?(diff_line) }
  end
end

if violations.empty?
  puts "fmt-check: changed lines are rustfmt-clean (#{changed.size} file(s) checked)."
  exit 0
end

warn "fmt-check: these changed lines are not rustfmt-formatted — run `rake format`:"
violations.uniq.each { |v| warn "  #{v}" }
warn "(Only lines you changed are checked; pre-existing drift elsewhere is ignored.)"
exit 1
