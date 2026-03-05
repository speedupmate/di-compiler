<?php
namespace Magento\Framework\App;

final class Registry
{
    private static array $data = [];

    public function __construct(
        \Magento\Framework\App\Config $config
    ) {}

    public function register(string $key, mixed $value): void
    {
        self::$data[$key] = $value;
    }
}
