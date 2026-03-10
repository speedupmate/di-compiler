<?php

declare(strict_types=1);

/**
 * Compare generated metadata parity between truth (_metadata) and output (metadata).
 *
 * Usage:
 *   php compare_metadata_parity.php
 *   php compare_metadata_parity.php --area=global
 *   php compare_metadata_parity.php --truth=/path/_metadata --output=/path/metadata --max-samples=20
 */

$opts = getopt('', ['area::', 'truth::', 'output::', 'max-samples::']);

$truthRoot = $opts['truth'] ?? '/var/www/application/generated/_metadata';
$outputRoot = $opts['output'] ?? '/var/www/application/generated/metadata';
$maxSamples = isset($opts['max-samples']) ? max(1, (int)$opts['max-samples']) : 15;
$areas = isset($opts['area']) ? [$opts['area']] : ['global', 'frontend', 'adminhtml', 'crontab', 'webapi_rest', 'webapi_soap', 'graphql'];
$sections = ['arguments', 'preferences', 'instanceTypes'];

function is_list_array(array $arr): bool {
    if ($arr === []) {
        return true;
    }
    return array_keys($arr) === range(0, count($arr) - 1);
}

function normalize_value($value) {
    if (!is_array($value)) {
        return $value;
    }
    // Preserve list ordering; normalize associative maps by key for stable compare.
    if (!is_list_array($value)) {
        ksort($value);
    }
    foreach ($value as $k => $v) {
        $value[$k] = normalize_value($v);
    }
    return $value;
}

function compare_nodes($truth, $out, string $path, array &$stats): void {
    if (is_array($truth) && is_array($out)) {
        $truthKeys = array_keys($truth);
        $outKeys = array_keys($out);

        $missingKeys = array_diff($truthKeys, $outKeys);
        foreach ($missingKeys as $k) {
            $p = $path === '' ? (string)$k : $path . '.' . $k;
            $stats['missing']++;
            if (count($stats['missing_samples']) < $stats['max_samples']) {
                $stats['missing_samples'][] = $p;
            }
        }

        $extraKeys = array_diff($outKeys, $truthKeys);
        foreach ($extraKeys as $k) {
            $p = $path === '' ? (string)$k : $path . '.' . $k;
            $stats['extra']++;
            if (count($stats['extra_samples']) < $stats['max_samples']) {
                $stats['extra_samples'][] = $p;
            }
        }

        $commonKeys = array_intersect($truthKeys, $outKeys);
        foreach ($commonKeys as $k) {
            $p = $path === '' ? (string)$k : $path . '.' . $k;
            compare_nodes($truth[$k], $out[$k], $p, $stats);
        }
        return;
    }

    if (gettype($truth) !== gettype($out)) {
        $stats['mismatches']++;
        if (count($stats['mismatch_samples']) < $stats['max_samples']) {
            $stats['mismatch_samples'][] = sprintf('%s [%s != %s]', $path, gettype($truth), gettype($out));
        }
        return;
    }

    $tn = normalize_value($truth);
    $on = normalize_value($out);
    if ($tn !== $on) {
        $stats['mismatches']++;
        if (count($stats['mismatch_samples']) < $stats['max_samples']) {
            $stats['mismatch_samples'][] = $path;
        }
    }
}

function print_samples(string $label, array $samples): void {
    if (!$samples) {
        return;
    }
    echo "  {$label}:\n";
    foreach ($samples as $s) {
        echo "    - {$s}\n";
    }
}

foreach ($areas as $area) {
    $truthFile = rtrim($truthRoot, '/') . '/' . $area . '.php';
    $outputFile = rtrim($outputRoot, '/') . '/' . $area . '.php';

    if (!is_file($truthFile) || !is_file($outputFile)) {
        fwrite(STDERR, "skip {$area}: missing file(s)\n");
        continue;
    }

    $truth = include $truthFile;
    $out = include $outputFile;

    echo "=== {$area} ===\n";

    $total = ['missing' => 0, 'extra' => 0, 'mismatches' => 0];

    foreach ($sections as $section) {
        $stats = [
            'missing' => 0,
            'extra' => 0,
            'mismatches' => 0,
            'missing_samples' => [],
            'extra_samples' => [],
            'mismatch_samples' => [],
            'max_samples' => $maxSamples,
        ];

        $tSection = $truth[$section] ?? [];
        $oSection = $out[$section] ?? [];

        compare_nodes($tSection, $oSection, $section, $stats);

        $total['missing'] += $stats['missing'];
        $total['extra'] += $stats['extra'];
        $total['mismatches'] += $stats['mismatches'];

        printf(
            "  %s: missing=%d, extra=%d, mismatches=%d\n",
            $section,
            $stats['missing'],
            $stats['extra'],
            $stats['mismatches']
        );

        print_samples('missing sample', $stats['missing_samples']);
        print_samples('extra sample', $stats['extra_samples']);
        print_samples('mismatch sample', $stats['mismatch_samples']);
    }

    printf(
        "  TOTAL: missing=%d, extra=%d, mismatches=%d\n\n",
        $total['missing'],
        $total['extra'],
        $total['mismatches']
    );
}
