<?php
namespace Magento\Framework\Event;

class Observer
{
    public function __construct(
        \Magento\Framework\EventManagerInterface $eventManager,
        \Magento\Framework\App\Config $config,
        \Magento\Framework\Logger\Handler ...$handlers
    ) {}
}
